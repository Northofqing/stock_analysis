use crate::agent::tool::Tool;
use crate::data_gateway::{GatewayBatch, ResearchDataGateway};
use async_trait::async_trait;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default)]
pub struct FetchResearchTool;

impl FetchResearchTool {
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FetchResearchTool {
    fn name(&self) -> &str {
        "fetch_research"
    }

    fn description(&self) -> &str {
        "获取指定 A 股的最新机构研报与评级；数据由统一 ResearchDataGateway 获取。"
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
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' parameter"))?;
        let batch = ResearchDataGateway::new()
            .instrument_reports(code, 20)
            .await?;
        let evidence = batch.evidence();
        let reports = match &batch {
            GatewayBatch::Available { records, .. } => records,
            GatewayBatch::VerifiedEmpty(_) => {
                anyhow::bail!(
                    "统一研报源已验证为空: code={code} provider={:?} source={} \
                     observed_at={} batch_id={}",
                    evidence.provider,
                    evidence.source,
                    evidence.observed_at,
                    evidence.batch_id
                )
            }
        };

        let mut rating_counts = BTreeMap::<String, usize>::new();
        for report in reports {
            if let Some(rating) = &report.rating {
                *rating_counts.entry(rating.clone()).or_default() += 1;
            }
        }
        let rows = reports
            .iter()
            .map(|report| {
                json!({
                    "title": report.title,
                    "institution": report.organization,
                    "rating": report.rating,
                    "rating_change": null,
                    "date": report.published_at,
                    "researcher": report.author,
                    "industry": report.industry_name,
                    "industry_code": report.industry_code,
                    "info_code": report.report_id,
                    "canonical_url": report.canonical_url,
                    "pdf_url": report.pdf_url,
                    "summary": null,
                })
            })
            .collect::<Vec<_>>();

        Ok(json!({
            "fetched": true,
            "code": code,
            "report_count": rows.len(),
            "summary_fetched": 0,
            "rating_distribution": rating_counts,
            "reports": rows,
            "evidence": {
                "provider": format!("{:?}", evidence.provider),
                "source": evidence.source,
                "source_at": evidence.source_at,
                "observed_at": evidence.observed_at,
                "batch_id": evidence.batch_id,
            },
            "note": "统一 ResearchDataGateway 返回结构化研报。当前已发布合同不提供正文摘要与 rating_change，字段显式为空。",
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::FetchResearchTool;

    #[test]
    fn tool_is_stateless() {
        let _ = FetchResearchTool::new();
    }
}
