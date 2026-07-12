use crate::analyzer::{AgentMode, GeminiAnalyzer};
use crate::signal::market_event::{Direction, EventType};
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
             {{\"is_event\":true/false,\"event_type\":\"Policy|TechBreak|OrderWin|Capacity|PriceUp|PriceDown|Mna|Accident|Overseas\",\"direction\":\"Bull|Neutral|Bear\",\"subject\":\"受益方\",\"confidence\":0.5-1.0}}\n\
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

    /// 修复 v9.1 集成: 真接 AI
    /// 失败 → 返回非事件输出 (保守, 不编造)
    pub async fn classify_with(
        gemini: &GeminiAnalyzer,
        title: &str,
        body: &str,
    ) -> ClassifierOutput {
        let prompt = Self::build_prompt(title, body);
        match gemini
            .call_api_mode(
                &prompt,
                "你是 A 股事件分类专家。只输出 JSON。",
                AgentMode::Quick,
            )
            .await
        {
            Ok(text) => Self::parse_response(&text).unwrap_or(ClassifierOutput {
                is_event: false,
                event_type: None,
                direction: None,
                subject: None,
                confidence: 0.0,
            }),
            Err(_) => ClassifierOutput {
                is_event: false,
                event_type: None,
                direction: None,
                subject: None,
                confidence: 0.0,
            },
        }
    }
}
