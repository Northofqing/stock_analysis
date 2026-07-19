use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct ContextManager {
    /// KV pairs of facts/data gathered during the session
    pub facts: HashMap<String, Value>,
    /// The scratchpad logging the thoughts and tools used
    pub scratchpad: Vec<String>,
}

impl ContextManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_fact(&mut self, key: &str, value: Value) {
        self.facts.insert(key.to_string(), value);
    }

    pub fn get_fact(&self, key: &str) -> Option<&Value> {
        self.facts.get(key)
    }

    pub fn remove_fact(&mut self, key: &str) -> Option<Value> {
        self.facts.remove(key)
    }

    pub fn log_event(&mut self, event: &str) {
        self.scratchpad.push(event.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_tool_can_remove_stale_fact() {
        let mut context = ContextManager::new();
        context.insert_fact(
            "fetch_financials",
            serde_json::json!({"gross_margin": 30.0}),
        );
        let removed = context.remove_fact("fetch_financials");
        assert!(removed.is_some());
        assert!(context.get_fact("fetch_financials").is_none());
    }
}
