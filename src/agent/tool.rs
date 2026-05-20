use std::future::Future;
use std::pin::Pin;
use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait Tool: Send + Sync {
    /// The exact name of the tool (must match what LLM expects)
    fn name(&self) -> &str;
    
    /// Description of the tool's purpose
    fn description(&self) -> &str;
    
    /// JSON Schema string or Value representing the function parameters
    fn parameters(&self) -> Value;
    
    /// Execute the tool with given arguments
    async fn call(&self, input: Value) -> anyhow::Result<String>;
}
