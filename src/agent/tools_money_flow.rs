use crate::agent::tool::Tool;
use crate::capital_flow::{format_for_prompt, IntradayShape, MoneyFlowSummary};
use crate::data_gateway::{CompanyDataGateway, GatewayBatch};
use crate::data_provider::service::service;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

pub struct FetchFundFlowTool;

impl Default for FetchFundFlowTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchFundFlowTool {
    pub fn new() -> Self {
        Self
    }
}

fn format_complete_fund_flow_context(
    code: &str,
    flow_result: anyhow::Result<Arc<MoneyFlowSummary>>,
    shape_result: anyhow::Result<Arc<IntradayShape>>,
    float_market_cap_yuan: Option<f64>,
) -> anyhow::Result<String> {
    let flow = flow_result
        .map_err(|error| anyhow::anyhow!("[{code}] money-flow batch unavailable: {error:#}"))?;
    // 分时形态是补充证据；它失败不应把已经拿到的真实资金流一起丢掉。
    // 缺失必须显式标注，不能静默当成“形态正常”。
    let (shape, shape_missing) = match shape_result {
        Ok(shape) => (shape, None),
        Err(error) => {
            log::warn!("[{code}] 日内形态缺失，保留资金流证据: {error:#}");
            (
                Arc::new(IntradayShape::default()),
                Some(format!("[数据缺失: IntradayShape] {error:#}")),
            )
        }
    };

    let mut prompt = format_for_prompt(&flow, &shape, float_market_cap_yuan);
    if let Some(note) = shape_missing {
        prompt.push_str(&format!("\n{note}\n"));
    }
    if prompt.trim().is_empty() {
        anyhow::bail!("No fund flow data found for {code}")
    }
    Ok(prompt)
}

#[async_trait]
impl Tool for FetchFundFlowTool {
    fn name(&self) -> &str {
        "fetch_fund_flow"
    }

    fn description(&self) -> &str {
        "获取指定 A 股近期主力资金净流入/流出情况（超级大单、大单）及今日日内分时走势形态，判断主力资金是否在真实介入或是诱多出逃。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "股票代码，如 '600519' 或 '000001'"
                }
            },
            "required": ["code"]
        })
    }

    async fn call(&self, input: serde_json::Value) -> anyhow::Result<String> {
        let code = input
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' parameter"))?;

        let svc = service();
        let gateway = CompanyDataGateway::new();
        let codes = vec![code.to_string()];
        // 20 日窗口用于四周期（5/10/20）判断；流通市值用于“净流入占流通市值”量级归一。
        let (flow_result, shape_result, stats_result) = tokio::join!(
            svc.get_money_flow(code, 20),
            svc.get_intraday_shape(code),
            gateway.market_statistics(&codes),
        );
        let float_market_cap = match stats_result {
            Ok(GatewayBatch::Available { records, .. }) => records
                .first()
                .and_then(|stats| stats.floating_market_cap().or_else(|| stats.total_market_cap()))
                .map(|value| value.get()),
            _ => None,
        };
        format_complete_fund_flow_context(code, flow_result, shape_result, float_market_cap)
    }
}

#[cfg(test)]
mod tests {
    use super::format_complete_fund_flow_context;
    use crate::capital_flow::{IntradayShape, MoneyFlowDay, MoneyFlowSummary};
    use std::sync::Arc;

    #[test]
    fn intraday_gateway_failure_keeps_fund_flow_and_marks_shape_missing() {
        let flow = MoneyFlowSummary {
            days: vec![MoneyFlowDay {
                date: "2026-07-27".to_string(),
                main_net: 100_000_000.0,
                xl_net: 60_000_000.0,
                big_net: 40_000_000.0,
                main_pct: 5.0,
                pct_chg: Some(2.0),
            }],
        };

        let rendered = format_complete_fund_flow_context(
            "TEST_CODE_600001",
            Ok(Arc::new(flow)),
            Err(anyhow::anyhow!(
                "Magic TDX intraday-shape gateway unavailable"
            )),
            Some(20_000_000_000.0),
        )
        .expect("real fund flow must survive a supplemental shape failure");

        assert!(rendered.contains("主力资金流向"));
        assert!(rendered.contains("[数据缺失: IntradayShape]"));
        assert!(rendered.contains("Magic TDX intraday-shape gateway unavailable"));
        // 1 天 1 亿 / 流通市值 200 亿 = +0.50%
        assert!(rendered.contains("5日 +0.50%"));
    }

    #[test]
    fn admitted_intraday_shape_is_rendered_with_the_money_flow_context() {
        let flow = MoneyFlowSummary {
            days: vec![MoneyFlowDay {
                date: "2026-07-29".to_string(),
                main_net: 100_000_000.0,
                xl_net: 60_000_000.0,
                big_net: 40_000_000.0,
                main_pct: 5.0,
                pct_chg: None,
            }],
        };
        let shape = IntradayShape {
            date: "2026-07-29".to_string(),
            pre_close: 10.0,
            open_pct: 1.0,
            high_pct: 3.0,
            low_pct: -1.0,
            close_pct: 2.0,
            amplitude: 4.0,
            tail_30m_pct: Some(0.8),
            shape_label: "TEST_CODE_稳步推高",
            present: true,
        };

        let rendered = format_complete_fund_flow_context(
            "TEST_CODE_600001",
            Ok(Arc::new(flow)),
            Ok(Arc::new(shape)),
            None,
        )
        .expect("complete admitted context");

        assert!(rendered.contains("主力资金流向"));
        assert!(rendered.contains("日内分时形态"));
        assert!(rendered.contains("TEST_CODE_稳步推高"));
    }
}
