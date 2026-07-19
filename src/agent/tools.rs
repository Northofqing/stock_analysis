use crate::agent::tool::Tool;
use crate::data_provider::service::service;
use async_trait::async_trait;
use serde_json::json;
// Replace standard serde_json::Value with local alias or fully qualify to avoid type collisions.

pub struct FetchFinancialTool;

impl Default for FetchFinancialTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchFinancialTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FetchFinancialTool {
    fn name(&self) -> &str {
        "fetch_financials"
    }

    fn description(&self) -> &str {
        "获取指定 A 股的最新一期核心财务指标，包含每股收益、ROE、毛利率、净利率、营收同比和净利润同比。"
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

        let fin = service().get_financials(code).await?;

        if fin.any() {
            let result = json!({
                "report_date": fin.report_date,
                "eps": fin.eps,
                "roe": fin.roe,
                "revenue_yoy": fin.revenue_yoy,
                "net_profit_yoy": fin.net_profit_yoy,
                "gross_margin": fin.gross_margin,
                "net_margin": fin.net_margin,
                "source": fin.source
            });
            Ok(result.to_string())
        } else {
            anyhow::bail!("No financial records found for {code}")
        }
    }
}
