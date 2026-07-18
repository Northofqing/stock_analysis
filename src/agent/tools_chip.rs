use crate::agent::tool::Tool;
use crate::data_provider::chip_distribution::{compute_chip_distribution, format_for_prompt};
use crate::data_provider::service::service;
use async_trait::async_trait;
use serde_json::json;

pub struct FetchChipDistributionTool;

impl Default for FetchChipDistributionTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchChipDistributionTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FetchChipDistributionTool {
    fn name(&self) -> &str {
        "fetch_chip_distribution"
    }

    fn description(&self) -> &str {
        "获取指定 A 股的当前筹码分布（CYQ），计算出当前价位的获利盘比例、上方套牢盘密集区、平均成本以及当前股票是否存在阻力重压区。"
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

        // 经 DataFetchService 缓存：与 pipeline 共享 250 日 K 线
        let daily_data = match service().get_kline(code, 250).await {
            Ok(d) => d,
            Err(e) => anyhow::bail!("Failed to fetch daily data for {code}: {e}"),
        };

        if daily_data.is_empty() {
            anyhow::bail!("No K-line data for chip distribution: {code}");
        }

        let chip_dist = compute_chip_distribution(&daily_data);
        if chip_dist.present {
            Ok(format_for_prompt(&chip_dist))
        } else {
            anyhow::bail!("Failed to compute chip distribution for {code}")
        }
    }
}
