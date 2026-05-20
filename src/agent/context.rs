use std::collections::HashMap;
use serde_json::Value;

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

    pub fn log_event(&mut self, event: &str) {
        self.scratchpad.push(event.to_string());
    }
}
