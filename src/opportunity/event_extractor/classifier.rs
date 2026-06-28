use crate::signal::market_event::{EventType, Direction};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ClassifierOutput {
    pub is_event: bool,
    #[serde(default)]
    pub event_type: Option<EventType>,
    #[serde(default)]
    pub direction: Option<Direction>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub confidence: f64,
}

pub struct EventClassifier;

impl EventClassifier {
    /// Build Quick AI prompt for event classification
    pub fn build_prompt(title: &str, body: &str) -> String {
        let body_100: String = body.chars().take(100).collect();
        format!(
            "标题：{title}\n正文前 100 字：{body_100}\n\n\
             判断：这是事件新闻还是非事件新闻？\n\n\
             JSON 格式：\n\
             {{\"is_event\":true/false,\"event_type\":\"Policy|TechBreak|...\",\"direction\":\"Bull|Neutral|Bear\",\"subject\":\"受益方\",\"confidence\":0.5-1.0}}\n\
             非事件→is_event=false, 其余 null\n事件→填所有字段"
        )
    }

    /// Parse AI JSON response. Garbage → None (no panic).
    pub fn parse_response(response_text: &str) -> Option<ClassifierOutput> {
        let cleaned = response_text
            .trim()
            .trim_start_matches("```json")
            .trim_end_matches("```")
            .trim();
        serde_json::from_str::<ClassifierOutput>(cleaned).ok()
    }
}
