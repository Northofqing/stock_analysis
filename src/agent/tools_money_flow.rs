use crate::agent::tool::Tool;
use crate::data_provider::money_flow::format_for_prompt;
use crate::data_provider::service::service;
use async_trait::async_trait;
use serde_json::json;

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
        let flow_arc = flow_result?;
        let shape_arc = shape_result?;

        let prompt_str = format_for_prompt(&flow_arc, &shape_arc);
        if prompt_str.trim().is_empty() {
            anyhow::bail!("No fund flow data found for {code}")
        } else {
            Ok(prompt_str)
        }
    }
}
