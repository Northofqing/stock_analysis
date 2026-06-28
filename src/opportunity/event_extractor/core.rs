use crate::signal::market_event::{MarketEvent, EventType, Direction, compute_event_id};
use crate::signal::market_event::SourceRef;
use super::adapter::{RawNewsItem, SourceType};
use super::classifier::ClassifierOutput;
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
    #[serde(default)]
    reason: Option<String>,
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
        let cleaned = response.trim().trim_start_matches("```json").trim_end_matches("```").trim();
        let dr: DeepResponse = serde_json::from_str(cleaned).ok()?;
        let event_id = compute_event_id(&item.title, &item.published_at);
        let mut me = MarketEvent::new(
            dr.event_type, dr.subject, dr.object,
            dr.direction, dr.strength, dr.certainty,
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
        let mut me = MarketEvent::new(
            event_type,
            co.subject.clone().unwrap_or_default(),
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
        let mut me = MarketEvent::new(
            event_type.unwrap_or(EventType::Other),
            item.title.chars().take(30).collect(),
            None,
            Direction::Neutral,
            30,
            30,
        );
        me.event_id = event_id;
        me.ai_degraded = true;
        me
    }
}

pub fn strength_for_event_type(et: EventType, confidence: f64) -> u8 {
    let base = match et {
        EventType::Policy => 75, EventType::TechBreak => 65,
        EventType::OrderWin => 60, EventType::Capacity => 55,
        EventType::PriceUp | EventType::PriceDown => 50,
        EventType::Mna => 60, EventType::Accident => 70,
        EventType::Overseas => 60, EventType::Other => 40,
    };
    let raw = (base as f64 * confidence).round() as u8;
    raw.clamp(20, 80)
}

pub fn certainty_for_source(st: SourceType, confidence: f64) -> u8 {
    let factor = match st {
        SourceType::Announcement => 1.0,
        SourceType::Search => 0.9,
        SourceType::Flash => 0.8,
    };
    let raw = (confidence * 100.0 * factor).round() as u8;
    raw.clamp(30, 85)
}
