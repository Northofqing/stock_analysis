use super::adapter::{RawNewsItem, SourceType};
use super::classifier::ClassifierOutput;
use crate::analyzer::{AgentMode, GeminiAnalyzer};
use crate::signal::market_event::SourceRef;
use crate::signal::market_event::{compute_event_id, Direction, EventType, MarketEvent};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
struct DeepResponse {
    event_type: EventType,
    direction: Direction,
    subject: String,
    #[serde(default)]
    object: Option<String>,
    strength: u8,
    certainty: u8,
    #[serde(default, rename = "reason")]
    _reason: Option<String>,
}

pub struct EventExtractorCore;

impl EventExtractorCore {
    pub fn build_prompt(title: &str, body: &str) -> String {
        format!(
            "标题：{title}\n正文：{body}\n\n\
            抽取完整 MarketEvent:\n\n\
            JSON 格式：\n\
            {{\"event_type\":\"...\",\"direction\":\"Bull|Neutral|Bear\",\"subject\":\"受益方\",\"object\":null,\"strength\":0-100,\"certainty\":0-100,\"reason\":\"原因\"}}\n\n\
            strength: 国家=80-100, 行业=50-79, 公司=20-49, 传闻=10-19\n\
            certainty: 官方=80-100, 深度报道=50-79, 快讯=20-49, 社交=0-19"
        )
    }

    pub fn parse_deep_response(item: &RawNewsItem, response: &str) -> Option<MarketEvent> {
        let cleaned = response
            .trim()
            .trim_start_matches("```json")
            .trim_end_matches("```")
            .trim();
        let dr: DeepResponse = serde_json::from_str(cleaned).ok()?;
        let event_id = compute_event_id(&item.title, &item.published_at);
        let mut me = MarketEvent::new_with_title(
            dr.event_type,
            dr.subject,
            item.title.clone(), // FIX-1: full_title 用完整 title
            dr.object,
            dr.direction,
            dr.strength,
            dr.certainty,
        );
        me.event_id = event_id;
        me.provenance.push(SourceRef {
            provider: item.source.clone(),
            url: item.url.clone(),
            fetched_at: item.published_at,
        });
        Some(me)
    }

    pub fn from_quick_only(item: &RawNewsItem, co: &ClassifierOutput) -> MarketEvent {
        let event_type = co.event_type.unwrap_or(EventType::Other);
        let strength = strength_for_event_type(event_type, co.confidence);
        let certainty = certainty_for_source(item.source_type, co.confidence);
        let event_id = compute_event_id(&item.title, &item.published_at);
        let mut me = MarketEvent::new_with_title(
            event_type,
            co.subject.clone().unwrap_or_default(),
            item.title.clone(), // FIX-1: full_title 用完整 title
            None,
            co.direction.unwrap_or(Direction::Neutral),
            strength,
            certainty,
        );
        me.event_id = event_id;
        me.provenance.push(SourceRef {
            provider: item.source.clone(),
            url: item.url.clone(),
            fetched_at: item.published_at,
        });
        me
    }

    pub fn build_degraded(item: &RawNewsItem, event_type: Option<EventType>) -> MarketEvent {
        let event_id = compute_event_id(&item.title, &item.published_at);
        let mut me = MarketEvent::new_with_title(
            event_type.unwrap_or(EventType::Other),
            item.title.chars().take(30).collect(),
            item.title.clone(), // FIX-1: full_title 用完整 title 而非 30 字符截断
            None,
            Direction::Neutral,
            30,
            30,
        );
        me.event_id = event_id;
        me.ai_degraded = true;
        me
    }

    /// 修复 v9.1 集成: 真接 AI (Deep 模式)
    /// 失败 → 退化为 build_degraded + ai_degraded=true
    pub async fn extract_with(gemini: &GeminiAnalyzer, item: &RawNewsItem) -> MarketEvent {
        let prompt = Self::build_prompt(&item.title, &item.body);
        match gemini
            .call_api_mode(
                &prompt,
                "你是 A 股量化事件抽取专家。只输出 JSON。",
                AgentMode::Deep,
            )
            .await
        {
            Ok(text) => Self::parse_deep_response(item, &text)
                .unwrap_or_else(|| Self::build_degraded(item, None)),
            Err(_) => Self::build_degraded(item, None),
        }
    }
}

pub fn strength_for_event_type(et: EventType, confidence: f64) -> u8 {
    let base = match et {
        EventType::Policy => 75,
        EventType::TechBreak => 65,
        EventType::OrderWin => 60,
        EventType::Capacity => 55,
        EventType::PriceUp | EventType::PriceDown => 50,
        EventType::Mna => 60,
        EventType::Accident => 70,
        EventType::Overseas => 60,
        // v15.3 D2.1: 4 路新源
        EventType::Earnings => 70,
        EventType::MarketAction => 80,
        EventType::AnalystView => 60,
        EventType::Announcement => 65,
        EventType::Other => 40,
    };
    let raw = (base as f64 * confidence).round() as u8;
    raw.clamp(20, 80)
}

pub fn certainty_for_source(st: SourceType, confidence: f64) -> u8 {
    // 修复 P0-1 校准区间: spec §4.2 要求 certainty 落在 source 对应的区间
    //   - Announcement (官方公告/交易所) → 80-100
    //   - Search       (财经媒体深度报道) → 50-79
    //   - Flash        (快讯/电报)         → 20-49
    //   - (Social     社交/传闻         → 0-19)  // 暂未实现
    // 之前: factor × confidence, 落到 40-85 区间, 与 spec 不符 (Flash 应该 ≤49)
    // 现在: linear map 让 confidence ∈ [0.5, 1.0] 落到对应区间
    let raw = match st {
        SourceType::Announcement => 60.0 + 40.0 * confidence, // [80, 100]
        SourceType::Search => 21.0 + 58.0 * confidence,       // [50, 79]
        SourceType::Flash => -9.0 + 58.0 * confidence,        // [20, 49]
    };
    raw.clamp(0.0, 100.0).round() as u8
}
