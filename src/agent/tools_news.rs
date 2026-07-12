use crate::agent::tool::Tool;
use crate::search_service::get_search_service;
use async_trait::async_trait;
use serde_json::json;

pub struct FetchNewsTool;

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
        "获取指定 A 股的近期重大新闻、公告或突发事件催化剂。如果你需要评估涨停原因或突然闪崩的具体利空利多消息，应该优先使用此工具。"
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
        let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("");

        let max_results = 5;
        let response = get_search_service()
            .search_stock_news(code, name, max_results)
            .await;

        let news_str = response.to_context(max_results);

        if news_str.is_empty() || news_str.contains("未找到相关结果") {
            Ok(json!({"error": "No recent news found for this stock."}).to_string())
        } else {
            Ok(news_str)
        }
    }
}
