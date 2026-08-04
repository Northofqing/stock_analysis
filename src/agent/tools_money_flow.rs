use crate::agent::tool::Tool;
use crate::capital_flow::{format_for_prompt, IntradayShape, MoneyFlowSummary};
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
) -> anyhow::Result<String> {
    let flow = flow_result
        .map_err(|error| anyhow::anyhow!("[{code}] money-flow batch unavailable: {error:#}"))?;
    let shape = shape_result.map_err(|error| {
        anyhow::anyhow!("[{code}] intraday money-flow shape unavailable: {error:#}")
    })?;

    let prompt = format_for_prompt(&flow, &shape);
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
        let (flow_result, shape_result) =
            tokio::join!(svc.get_money_flow(code, 5), svc.get_intraday_shape(code),);
        format_complete_fund_flow_context(code, flow_result, shape_result)
    }
}

#[cfg(test)]
mod tests {
    use super::format_complete_fund_flow_context;
    use crate::capital_flow::{IntradayShape, MoneyFlowDay, MoneyFlowSummary};
    use std::sync::Arc;

    #[test]
    fn intraday_gateway_failure_rejects_the_complete_agent_context() {
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

        let error = format_complete_fund_flow_context(
            "TEST_CODE_600001",
            Ok(Arc::new(flow)),
            Err(anyhow::anyhow!(
                "Magic TDX intraday-shape gateway unavailable"
            )),
        )
        .expect_err("gateway failure must reject the complete context");

        let message = error.to_string();
        assert!(message.contains("TEST_CODE_600001"));
        assert!(message.contains("Magic TDX"));
        assert!(message.contains("intraday money-flow shape"));
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
        )
        .expect("complete admitted context");

        assert!(rendered.contains("主力资金流向"));
        assert!(rendered.contains("日内分时形态"));
        assert!(rendered.contains("TEST_CODE_稳步推高"));
    }
}
