//! BR-118 capital-flow analysis domain.
//!
//! Acquisition belongs to [`crate::data_gateway::CapitalDataGateway`].  This
//! module only projects the gateway's complete, audited facts into the small
//! domain shape consumed by scoring and prompt rendering.

use anyhow::{bail, Result};
use magic_market_core::FlowInterval;
use serde::{Deserialize, Serialize};

use crate::data_gateway::{GatewayBatch, InstrumentFundFlowFact};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoneyFlowDay {
    pub date: String,
    pub main_net: f64,
    pub xl_net: f64,
    pub big_net: f64,
    pub main_pct: f64,
    /// The unified fund-flow contract does not carry price change.
    ///
    /// Keep the absence explicit.  Consumers that require price change must
    /// evaluate an independently audited price batch; this projection must not
    /// join unrelated provider batches.
    pub pct_chg: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MoneyFlowSummary {
    pub days: Vec<MoneyFlowDay>,
}

impl MoneyFlowSummary {
    pub fn from_gateway(batch: GatewayBatch<InstrumentFundFlowFact>) -> Result<Self> {
        let records = match batch {
            GatewayBatch::Available { records, .. } => records,
            GatewayBatch::VerifiedEmpty(_) => {
                bail!("[BR-164] capital gateway returned a verified-empty batch")
            }
        };
        if records.is_empty() {
            bail!("[BR-164] capital gateway returned an empty admitted batch");
        }
        if records
            .iter()
            .any(|record| record.interval != FlowInterval::Day1)
        {
            bail!("[BR-164] daily capital projection received a non-daily fact");
        }
        Ok(Self {
            days: records
                .into_iter()
                .map(|record| MoneyFlowDay {
                    date: record.period_at,
                    main_net: record.main_net,
                    xl_net: record.super_large_net,
                    big_net: record.large_net,
                    main_pct: record.main_ratio_percent,
                    pct_chg: None,
                })
                .collect(),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.days.is_empty()
    }

    pub fn latest(&self) -> Option<&MoneyFlowDay> {
        self.days.last()
    }

    pub fn recent_main_sum(&self, n: usize) -> f64 {
        let start = self.days.len().saturating_sub(n);
        self.days[start..].iter().map(|day| day.main_net).sum()
    }

    pub fn ewma_main_net_yi(&self) -> Option<f64> {
        if self.days.is_empty() {
            return None;
        }
        const WEIGHTS: [f64; 5] = [0.1, 0.1, 0.15, 0.25, 0.4];
        let take = self.days.len().min(5);
        let days = &self.days[self.days.len() - take..];
        let weights = &WEIGHTS[5 - take..];
        let total_weight: f64 = weights.iter().sum();
        Some(
            days.iter()
                .zip(weights)
                .map(|(day, weight)| day.main_net * weight)
                .sum::<f64>()
                / total_weight
                / 1e8,
        )
    }

    pub fn is_one_day_bounce(&self) -> bool {
        let sum_five = self.recent_main_sum(5) / 1e8;
        let Some(latest) = self.latest() else {
            return false;
        };
        let latest = latest.main_net / 1e8;
        sum_five < -30.0 && latest > 0.0 && latest < (-sum_five) * 0.2
    }
}

#[derive(Debug, Clone, Default)]
pub struct IntradayShape {
    pub date: String,
    pub pre_close: f64,
    pub open_pct: f64,
    pub high_pct: f64,
    pub low_pct: f64,
    pub close_pct: f64,
    pub amplitude: f64,
    pub tail_30m_pct: Option<f64>,
    pub shape_label: &'static str,
    pub present: bool,
}

pub fn format_for_prompt(flow: &MoneyFlowSummary, shape: &IntradayShape) -> String {
    let mut output = String::new();
    if !flow.is_empty() {
        output.push_str("\n【主力资金流向（真实口径，单位：亿元）】\n");
        output.push_str("日期 | 涨跌幅% | 主力净流入 | 主力占比% | 超大单 | 大单\n");
        for day in flow
            .days
            .iter()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .iter()
            .rev()
        {
            let pct_chg = day
                .pct_chg
                .map(|value| format!("{value:+.2}%"))
                .unwrap_or_else(|| "缺失".to_string());
            output.push_str(&format!(
                "{} | {} | {:+.2} | {:+.2}% | {:+.2} | {:+.2}\n",
                day.date,
                pct_chg,
                day.main_net / 1e8,
                day.main_pct,
                day.xl_net / 1e8,
                day.big_net / 1e8,
            ));
        }
        output.push_str(&format!(
            "近3日主力累计净流入: {:+.2}亿 | 近5日: {:+.2}亿\n",
            flow.recent_main_sum(3) / 1e8,
            flow.recent_main_sum(5) / 1e8
        ));
    }
    if shape.present {
        output.push_str("\n【日内分时形态】\n");
        output.push_str(&format!(
            "开盘{:+.2}% | 最高{:+.2}% | 最低{:+.2}% | 收盘{:+.2}%\n",
            shape.open_pct, shape.high_pct, shape.low_pct, shape.close_pct
        ));
        let tail = shape
            .tail_30m_pct
            .map(|value| format!("{value:+.2}%"))
            .unwrap_or_else(|| "暂无（未到14:30）".to_string());
        output.push_str(&format!(
            "日内振幅: {:.2}%  尾盘30分钟涨幅: {tail}\n日内形态: {}\n",
            shape.amplitude, shape.shape_label
        ));
    } else {
        output.push_str("\n【日内分时形态】数据缺失（统一契约尚无等价形态字段）\n");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::BatchEvidence;
    use crate::magic_compat::ProviderId;

    #[test]
    fn projection_keeps_missing_price_change_explicit() {
        let summary = MoneyFlowSummary::from_gateway(GatewayBatch::Available {
            records: vec![InstrumentFundFlowFact {
                code: "TEST_CODE_600519".to_string(),
                interval: FlowInterval::Day1,
                period_at: "2026-07-24".to_string(),
                main_net: 100.0,
                main_ratio_percent: 1.0,
                super_large_net: 40.0,
                large_net: 60.0,
                medium_net: -20.0,
                small_net: -80.0,
            }],
            evidence: BatchEvidence {
                provider: ProviderId::Eastmoney,
                source: "TEST_CODE_PROVIDER".to_string(),
                batch_id: "TEST_CODE_BATCH".to_string(),
                source_at: Some("2026-07-24".to_string()),
                observed_at: "2026-07-24T07:00:01Z".to_string(),
            },
        })
        .unwrap();

        assert_eq!(summary.days[0].pct_chg, None);
        assert!(format_for_prompt(&summary, &IntradayShape::default()).contains("缺失"));
    }
}
