use crate::agent::tool::Tool;
use crate::data_gateway::{grpc_source, GatewayBatch};
use async_trait::async_trait;
use serde_json::json;

pub struct FetchNewsTool;

impl Default for FetchNewsTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchNewsTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FetchNewsTool {
    fn name(&self) -> &str {
        "fetch_news"
    }

    fn description(&self) -> &str {
        "获取指定 A 股的近期个股新闻（gRPC InstrumentNews，近 7 天）。如果你需要评估涨停原因或突然闪崩的具体利空利多消息，应该优先使用此工具。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "股票代码，如 '600519' 或 '000001'"
                },
                "name": {
                    "type": "string",
                    "description": "股票中文简拼名称，如 '贵州茅台'。如果不确定可给空字符串。"
                }
            },
            "required": ["code", "name"]
        })
    }

    async fn call(&self, input: serde_json::Value) -> anyhow::Result<String> {
        let code = input
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' parameter"))?;

        let max_results = 5;
        let source = grpc_source::bridge_for("InstrumentNews")
            .map_err(|error| anyhow::anyhow!("InstrumentNews 网关不可用: {error}"))?;
        let batch = source
            .instrument_news_async(&[code.to_string()], 7)
            .await
            .map_err(|error| anyhow::anyhow!("InstrumentNews 查询失败: {error}"))?;
        let evidence = batch.evidence();
        let records = match &batch {
            GatewayBatch::Available { records, .. } => records,
            GatewayBatch::VerifiedEmpty(_) => anyhow::bail!(
                "[数据缺失: InstrumentNews/records] code={code} provider={:?} \
                 source={} batch_id={} observed_at={}",
                evidence.provider,
                evidence.source,
                evidence.batch_id,
                evidence.observed_at
            ),
        };
        if records.is_empty() {
            anyhow::bail!(
                "[数据缺失: InstrumentNews/records] code={code} provider={:?} batch_id={}",
                evidence.provider,
                evidence.batch_id
            );
        }

        let lines = records
            .iter()
            .take(max_results)
            .map(|record| {
                let item = record.persistence_item();
                let summary = if item.summary.trim().is_empty() {
                    "（无摘要）"
                } else {
                    item.summary.trim()
                };
                format!(
                    "- [{}] {}（{}）\n  {}\n  {}",
                    item.published_at.format("%Y-%m-%d %H:%M UTC"),
                    item.title,
                    item.source_name,
                    item.url,
                    summary
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(format!(
            "个股新闻（InstrumentNews，近 7 天，provider={:?} source={} batch_id={} observed_at={}）：\n{}",
            evidence.provider,
            evidence.source,
            evidence.batch_id,
            evidence.observed_at,
            lines
        ))
    }
}
