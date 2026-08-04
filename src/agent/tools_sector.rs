use crate::agent::tool::Tool;
use crate::data_gateway::{BoardDataGateway, BoardKind, GatewayBatch};
use async_trait::async_trait;
use serde_json::json;

#[derive(Debug, Clone, Copy, Default)]
pub struct FetchSectorTool;

impl FetchSectorTool {
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FetchSectorTool {
    fn name(&self) -> &str {
        "fetch_sector_concepts"
    }

    fn description(&self) -> &str {
        "获取指定 A 股的行业与概念板块；数据由统一 Magic TDX BoardDataGateway 获取。"
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
        let batch = BoardDataGateway::new().memberships(code).await?;
        let evidence = batch.evidence();
        let rows = match &batch {
            GatewayBatch::Available { records, .. } => records,
            GatewayBatch::VerifiedEmpty(_) => {
                anyhow::bail!(
                    "Magic TDX 板块归属已验证为空: code={code} provider={:?} source={} \
                     observed_at={} batch_id={}",
                    evidence.provider,
                    evidence.source,
                    evidence.observed_at,
                    evidence.batch_id
                )
            }
        };

        let industries = rows
            .iter()
            .filter(|row| row.kind == BoardKind::Industry)
            .map(|row| row.board_name.clone())
            .collect::<Vec<_>>();
        let concepts = rows
            .iter()
            .filter(|row| row.kind == BoardKind::Concept)
            .map(|row| row.board_name.clone())
            .collect::<Vec<_>>();
        let all_boards = rows
            .iter()
            .map(|row| row.board_name.clone())
            .collect::<Vec<_>>();

        Ok(json!({
            "fetched": true,
            "secucode": code,
            "primary_boards": industries,
            "secondary_boards": concepts,
            "all_boards": all_boards,
            "board_count": rows.len(),
            "memberships": rows.iter().map(|row| json!({
                "board_code": row.board_code,
                "board_name": row.board_name,
                "category": format!("{:?}", row.kind),
            })).collect::<Vec<_>>(),
            "evidence": {
                "provider": format!("{:?}", evidence.provider),
                "source": evidence.source,
                "source_at": evidence.source_at,
                "observed_at": evidence.observed_at,
                "batch_id": evidence.batch_id,
            },
            "note": "统一 Magic TDX 板块归属；行业/概念使用源类别，不再按列表位置猜测。",
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::FetchSectorTool;

    #[test]
    fn tool_is_stateless() {
        let _ = FetchSectorTool::new();
    }
}
