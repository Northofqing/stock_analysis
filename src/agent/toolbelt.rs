use crate::agent::tool::Tool;
use async_openai::types::{ChatCompletionTool, ChatCompletionToolType, FunctionObject};
use std::collections::HashMap;
use std::sync::Arc;

pub struct Toolbelt {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl Default for Toolbelt {
    fn default() -> Self {
        Self::new()
    }
}

impl Toolbelt {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: impl Tool + 'static) {
        self.tools.insert(tool.name().to_string(), Arc::new(tool));
    }

    /// Converts registered tools to `async_openai` format
    pub fn as_openai_tools(&self) -> Vec<ChatCompletionTool> {
        self.tools
            .values()
            .map(|t| ChatCompletionTool {
                r#type: ChatCompletionToolType::Function,
                function: FunctionObject {
                    name: t.name().to_string(),
                    description: Some(t.description().to_string()),
                    parameters: Some(t.parameters()),
                },
            })
            .collect()
    }

    pub async fn execute(&self, name: &str, input: serde_json::Value) -> anyhow::Result<String> {
        if let Some(tool) = self.tools.get(name) {
            tool.call(input).await
        } else {
            Err(anyhow::anyhow!("Tool {} not found", name))
        }
    }
}
