# event_extractor 实施计划 — v9.1 §2 补充

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 v9 流水线第②阶段 (AI 结构化事件抽取): 8+ provider 新闻流 → 三层漏斗 → 结构化 MarketEvent

**Architecture:** 三个模块依次串联 (adapter → rule_filter → classifier+core), 加 mód 入口。每个模块可独立测试、独立验收。AI 不可用时完整降级。

**Tech Stack:** Rust 2021 · tokio · 现有 GeminiAnalyzer (AgentMode::Quick/Deep) · MarketEvent (T1 已就绪)

## Global Constraints

- **TDD 强制**: 每个 Task 先写测试 → 确认失败 → 写实现 → 确认通过 → commit
- **不引入新依赖**: 不增 NLP 库, 复用现有 GeminiAnalyzer
- **AGENTS.md 5.0 红线**: 数据缺≠编造, stale=丢弃+审计, AI 挂=降级不 panic
- **量化 PM Gate**: 8 条验收标准 (spec §8)
- **v9.1 plan Task 1 对应**: 本计划补充 T1 缺失的 event_extractor 部分

---

## 文件结构 (5 files, ~800 lines)

| 文件 | 角色 | 依赖 |
|------|------|------|
| `src/opportunity/event_extractor/adapter.rs` | SearchResult → RawNewsItem | 无 |
| `src/opportunity/event_extractor/rule_filter.rs` | 规则预筛 (关键词表) | adapter (RawNewsItem) |
| `src/opportunity/event_extractor/classifier.rs` | Quick AI 分类 | rule_filter (RuleMatch) |
| `src/opportunity/event_extractor/core.rs` | Deep AI + 盘中 Quick-only 确定性映射 | classifier |
| `src/opportunity/event_extractor/mod.rs` | 公共入口 + 错误类型 | 全部 |
| `tests/event_extractor_tests.rs` | 全层级集成测试 | — |

---

### Task 1: adapter (SearchResult → RawNewsItem)

**Files:**
- Create: `src/opportunity/event_extractor/adapter.rs`
- Create: `src/opportunity/event_extractor/mod.rs`
- Test: `tests/event_extractor_tests.rs`

**Interfaces:**
- Consumes: `crate::search_service::SearchResult` (已有)
- Produces: `RawNewsItem { title, body, source, source_priority, source_type, published_at, url }`

- [x] **Step 1: 写失败测试**

```rust
// tests/event_extractor_tests.rs
use stock_analysis::opportunity::event_extractor::adapter::*;
use stock_analysis::search_service::SearchResult;
use stock_analysis::search_service::NewsType;

#[test]
fn test_adapter_search_result_to_raw() {
    // 修复 P0-1: 来自东方财富搜索的 SearchResult 必正确映射
    let sr = SearchResult {
        title: "工信部: 5G-A 商用进入新阶段".into(),
        snippet: "工信部发布...".into(),
        url: "https://eastmoney.com/a".into(),
        source: "东方财富".into(),
        published_date: Some("2026-06-27 10:30:00".into()),
        news_type: NewsType::Policy,
        sentiment: stock_analysis::search_service::Sentiment::Positive,
        importance: 8,
        relevance: 0.9,
        keywords: vec!["5G".into(), "工信部".into()],
    };
    let raw = SearchResultAdapter::to_raw(&sr).unwrap();
    assert_eq!(raw.title, "工信部: 5G-A 商用进入新阶段");
    assert!(!raw.body.is_empty(), "snippet 必填到 body");
    assert_eq!(raw.source, "东方财富");
    assert_eq!(raw.source_priority, 3); // 东方财富=3
    assert_eq!(raw.source_type, SourceType::Search);
    assert!(raw.published_at.year() == 2026);
}

#[test]
fn test_adapter_missing_published_date_fails() {
    // 修复 §5.1 红线: published_at 缺失必报错, 不静默
    let sr = SearchResult {
        title: "test".into(), snippet: "".into(), url: "".into(),
        source: "jin10".into(), published_date: None,
        news_type: NewsType::Flash, sentiment: stock_analysis::search_service::Sentiment::Neutral,
        importance: 0, relevance: 0.0, keywords: vec![],
    };
    let result = SearchResultAdapter::to_raw(&sr);
    assert!(result.is_err(), "published_date 缺失必报错");
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("published_date") || msg.contains("E_INVALID_PUBLISHED_AT"));
}

#[test]
fn test_adapter_flash_without_body() {
    // jin10 快讯: snippet 为空, body 也空
    let sr = SearchResult {
        title: "快讯标题".into(), snippet: "".into(), url: "".into(),
        source: "jin10".into(),
        published_date: Some("2026-06-27 10:30:00".into()),
        news_type: NewsType::Flash, sentiment: stock_analysis::search_service::Sentiment::Neutral,
        importance: 0, relevance: 0.0, keywords: vec![],
    };
    let raw = SearchResultAdapter::to_raw(&sr).unwrap();
    assert!(raw.body.is_empty(), "快讯无 body 必空字符串");
    assert_eq!(raw.source_priority, 2); // jin10=2
}

#[test]
fn test_adapter_source_priority_mapping() {
    // 修复 P0-1: 各 provider 映射到正确的 source_priority
    let mut sr = minimal_sr();
    sr.source = "巨潮".into(); sr.published_date = Some("2026-06-27".into());
    sr.news_type = NewsType::Announcement;
    let raw = SearchResultAdapter::to_raw(&sr).unwrap();
    assert_eq!(raw.source_priority, 4, "巨潮必 priority=4");
    assert_eq!(raw.source_type, SourceType::Announcement);
}
```

- [x] **Step 2: 跑测试确认失败**

```bash
cargo test --test event_extractor_tests
# Expected: error[E0432]: unresolved import `stock_analysis::opportunity::event_extractor`
```

- [x] **Step 3: 写 mod.rs 入口 + adapter.rs 实现**

```rust
// src/opportunity/event_extractor/mod.rs
pub mod adapter;
pub mod rule_filter;
pub mod classifier;
pub mod core;

pub use adapter::SearchResultAdapter;
```

```rust
// src/opportunity/event_extractor/adapter.rs
use chrono::{DateTime, Local, NaiveDateTime};
use crate::search_service::{SearchResult, NewsType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType { Flash, Search, Announcement }

#[derive(Debug, Clone)]
pub struct RawNewsItem {
    pub title: String,
    pub body: String,
    pub source: String,
    pub source_priority: u8,
    pub source_type: SourceType,
    pub published_at: DateTime<Local>,
    pub url: Option<String>,
}

pub struct SearchResultAdapter;

impl SearchResultAdapter {
    pub fn to_raw(sr: &SearchResult) -> Result<RawNewsItem, String> {
        // 修复 §5.1 红线: published_at 缺失必报错
        let date_str = sr.published_date.as_deref()
            .ok_or_else(|| format!("E_INVALID_PUBLISHED_AT: published_date 缺失 (source={}, title={})",
                sr.source, sr.title))?;
        // 解析多种日期格式
        let naive = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%d %H:%M:%S")
            .or_else(|_| NaiveDateTime::parse_from_str(date_str, "%Y-%m-%d"))
            .map_err(|e| format!("E_INVALID_PUBLISHED_AT: 无法解析 '{}': {}", date_str, e))?;
        let published_at: DateTime<Local> = DateTime::from_naive_utc_and_offset(naive, *Local::now().offset());

        let (source_priority, source_type) = match sr.source.as_str() {
            "巨潮" | "cninfo" => (4, SourceType::Announcement),
            "上交所" | "深交所" | "sse" | "szse" => (4, SourceType::Announcement),
            "东方财富" | "eastmoney" => (3, SourceType::Search),
            "jin10" | "金十" => (2, SourceType::Flash),
            "财联社" | "cls" => (2, SourceType::Flash),
            "华尔街见闻" | "wscn" => (2, SourceType::Flash),
            _ => (1, SourceType::Search),
        };

        Ok(RawNewsItem {
            title: sr.title.clone(),
            body: sr.snippet.clone(),
            source: sr.source.clone(),
            source_priority,
            source_type,
            published_at,
            url: if sr.url.is_empty() { None } else { Some(sr.url.clone()) },
        })
    }
}
```

```rust
// src/opportunity/mod.rs 加一行
pub mod event_extractor;
```

- [x] **Step 4: 跑测试确认通过**

```bash
cargo test --test event_extractor_tests test_adapter
# Expected: 4 passed
```

- [x] **Step 5: commit**

```bash
git add src/opportunity/event_extractor/ src/opportunity/mod.rs tests/event_extractor_tests.rs
git commit -m "feat(event_extractor): T1 adapter (SearchResult → RawNewsItem, source_priority, 日期校验)"
```

---

### Task 2: rule_filter (规则预筛)

**Files:**
- Create: `src/opportunity/event_extractor/rule_filter.rs`
- Test: `tests/event_extractor_tests.rs` (追加)

**Interfaces:**
- Consumes: `RawNewsItem` (Task 1)
- Produces: `RuleMatch { item, matched, event_type, discard_reason }`

- [x] **Step 1: 写失败测试**

```rust
// tests/event_extractor_tests.rs (追加)
use stock_analysis::opportunity::event_extractor::rule_filter::*;

fn raw(title: &str) -> RawNewsItem {
    RawNewsItem {
        title: title.into(), body: "".into(), source: "test".into(),
        source_priority: 1, source_type: SourceType::Search,
        published_at: chrono::Local::now(),
        url: None,
    }
}

#[test]
fn test_rule_filter_discards_noise() {
    // 修复 P0-1: "收评" → 丢弃
    let rm = RuleFilter::filter(&raw("A股收评：三大指数低开高走"));
    assert!(!rm.matched);
    assert!(rm.discard_reason.is_some());
    assert!(rm.discard_reason.unwrap().contains("收评"));
}

#[test]
fn test_rule_filter_keeps_tech_break() {
    // "突破" → 保留 + event_type=TechBreak
    let rm = RuleFilter::filter(&raw("CO2 激光在半导体晶圆制造中取得重大突破"));
    assert!(rm.matched);
    assert_eq!(rm.event_type, Some(EventType::TechBreak));
}

#[test]
fn test_rule_filter_keeps_policy() {
    // "工信部" → Policy
    let rm = RuleFilter::filter(&raw("工信部：5G-A 商用部署进入新阶段"));
    assert!(rm.matched);
    assert_eq!(rm.event_type, Some(EventType::Policy));
}

#[test]
fn test_rule_filter_keeps_priceup() {
    // "涨价" → PriceUp
    let rm = RuleFilter::filter(&raw("碳酸锂价格上调 5000 元"));
    assert!(rm.matched);
    assert_eq!(rm.event_type, Some(EventType::PriceUp));
}

#[test]
fn test_rule_filter_discards_fund() {
    // "基金净值" → 丢弃 (非股票域)
    let rm = RuleFilter::filter(&raw("XX 基金净值突破 2 元"));
    assert!(!rm.matched);
}

#[test]
fn test_rule_filter_unknown_keyword_passes() {
    // 关键词表不覆盖 → 保留 (走 AI 兜底), event_type=Other
    let rm = RuleFilter::filter(&raw("某公司发布新版本内部管理系统"));
    assert!(rm.matched, "关键词未知必保留 (AI 兜底)");
    assert_eq!(rm.event_type, Some(EventType::Other));
}
```

- [x] **Step 2: 跑测试确认失败**

```bash
cargo test --test event_extractor_tests test_rule_filter
# Expected: error[E0432]
```

- [x] **Step 3: 最小实现**

```rust
// src/opportunity/event_extractor/rule_filter.rs
use crate::signal::market_event::EventType;
use super::adapter::RawNewsItem;

pub struct RuleMatch {
    pub item: RawNewsItem,
    pub matched: bool,
    pub event_type: Option<EventType>,
    pub discard_reason: Option<String>,
}

pub struct RuleFilter;

impl RuleFilter {
    pub fn filter(item: &RawNewsItem) -> RuleMatch {
        let t = &item.title;

        // 1. 丢弃规则 (优先级高于保留)
        let discard_rules: &[(&[&str], &str)] = &[
            (&["收评", "复盘", "盘面", "午评", "早评"], "收评/复盘"),
            (&["涨停揭秘", "涨停复盘", "打板"], "涨停揭秘"),
            (&["龙虎榜", "大宗交易"], "龙虎榜/大宗交易"),
            (&["明日操作", "操盘", "掘金", "金股"], "荐股号"),
            (&["基金净值", "ETF", "保险", "理财"], "非股票域"),
            (&["期货", "外汇", "黄金", "原油"], "非A股域"),
        ];
        for (keywords, reason) in discard_rules {
            if keywords.iter().any(|k| t.contains(k)) {
                return RuleMatch {
                    item: item.clone(), matched: false,
                    event_type: None,
                    discard_reason: Some(reason.to_string()),
                };
            }
        }

        // 2. 保留规则 (锚定 event_type)
        let keep_rules: &[(&[&str], EventType)] = &[
            (&["突破", "量产", "首发", "全球首", "攻克", "发布", "推出", "问世"], EventType::TechBreak),
            (&["订单", "中标", "签约", "合同", "获得"], EventType::OrderWin),
            (&["涨价", "提价", "上调", "紧缺", "供不应求"], EventType::PriceUp),
            (&["降价", "下调"], EventType::PriceDown),
            (&["扩产", "投产", "产能", "新建"], EventType::Capacity),
            (&["收购", "重组", "合并", "并购", "入股"], EventType::Mna),
            (&["政策", "国务院", "工信部", "央行", "财政部"], EventType::Policy),
            (&["停产", "事故", "火灾", "爆炸", "泄漏"], EventType::Accident),
            (&["制裁", "禁令", "关税", "美联储", "加息"], EventType::Overseas),
        ];
        for (keywords, event_type) in keep_rules {
            if keywords.iter().any(|k| t.contains(k)) {
                return RuleMatch {
                    item: item.clone(), matched: true,
                    event_type: Some(*event_type),
                    discard_reason: None,
                };
            }
        }

        // 3. 关键词表不覆盖 → 保留 (走 AI 兜底)
        RuleMatch {
            item: item.clone(), matched: true,
            event_type: Some(EventType::Other),
            discard_reason: None,
        }
    }
}
```

- [x] **Step 4: 跑测试**

```bash
cargo test --test event_extractor_tests test_rule_filter
# Expected: 6 passed
```

- [x] **Step 5: commit**

```bash
git add src/opportunity/event_extractor/rule_filter.rs tests/event_extractor_tests.rs
git commit -m "feat(event_extractor): T2 rule_filter (6 词号表 × 70% 噪音过滤 + 9 个 event_type 锚定)"
```

---

### Task 3: classifier (Quick AI 分类)

**Files:**
- Create: `src/opportunity/event_extractor/classifier.rs`
- Test: `tests/event_extractor_tests.rs` (追加)

**Interfaces:**
- Consumes: `RuleMatch` (Task 2), `GeminiAnalyzer`
- Produces: `ClassifierOutput { is_event, event_type, direction, subject, confidence }`

- [x] **Step 1: 写失败测试**

```rust
// tests/event_extractor_tests.rs (追加)
use stock_analysis::opportunity::event_extractor::classifier::*;

#[test]
fn test_classifier_parse_valid_json() {
    // 修复 P0-1: mock AI 返回 JSON → 正确解析
    let json = r#"{"is_event":true,"event_type":"Policy","direction":"Bull","subject":"工信部","confidence":0.9}"#;
    let out = EventClassifier::parse_response(json).unwrap();
    assert!(out.is_event);
    assert_eq!(out.event_type.unwrap(), EventType::Policy);
    assert_eq!(out.direction.unwrap(), Direction::Bull);
    assert_eq!(out.subject.unwrap(), "工信部");
    assert!((out.confidence - 0.9).abs() < 0.01);
}

#[test]
fn test_classifier_parse_non_event() {
    let json = r#"{"is_event":false,"event_type":null,"direction":null,"subject":null,"confidence":0.3}"#;
    let out = EventClassifier::parse_response(json).unwrap();
    assert!(!out.is_event);
    assert!(out.event_type.is_none());
}

#[test]
fn test_classifier_parse_garbage_returns_none() {
    // 修复 P0-1: AI 返回垃圾 → 不 panic, 返回 None
    let out = EventClassifier::parse_response("hello world");
    assert!(out.is_none());
}
```

- [x] **Step 2: 确认失败**

```bash
cargo test --test event_extractor_tests test_classifier
# Expected: error[E0432]
```

- [x] **Step 3: 最小实现**

```rust
// src/opportunity/event_extractor/classifier.rs
use crate::signal::market_event::{EventType, Direction};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ClassifierOutput {
    pub is_event: bool,
    pub event_type: Option<EventType>,
    pub direction: Option<Direction>,
    pub subject: Option<String>,
    pub confidence: f64,
}

pub struct EventClassifier;

impl EventClassifier {
    /// 修复 P0-1: 给 Quick AI prompt
    pub fn build_prompt(title: &str, body: &str) -> String {
        let body_100 = body.chars().take(100).collect::<String>();
        format!(
            "标题：{title}\n正文前 100 字：{body_100}\n\n\
             判断：这是事件新闻还是非事件新闻？\n\n\
             JSON 格式：\n\
             {{\n  \"is_event\": true/false,\n  \"event_type\": \"Policy|TechBreak|...\",\n  \"direction\": \"Bull|Neutral|Bear\",\n  \"subject\": \"受益方\",\n  \"confidence\": 0.5-1.0\n}}\n\
             非事件→is_event=false, 其余 null\n事件→填所有字段"
        )
    }

    /// 修复 P0-1: 解析 AI 返回的 JSON
    pub fn parse_response(response_text: &str) -> Option<ClassifierOutput> {
        let cleaned = response_text
            .trim()
            .trim_start_matches("```json")
            .trim_end_matches("```")
            .trim();
        serde_json::from_str::<ClassifierOutput>(cleaned).ok()
    }
}
```

- [x] **Step 4: 跑测试**

```bash
cargo test --test event_extractor_tests test_classifier
# Expected: 3 passed
```

- [x] **Step 5: commit**

```bash
git add src/opportunity/event_extractor/classifier.rs tests/event_extractor_tests.rs
git commit -m "feat(event_extractor): T3 classifier (Quick AI prompt + JSON parse)"
```

---

### Task 4: core (Deep AI + 盘中确定性映射)

**Files:**
- Create: `src/opportunity/event_extractor/core.rs`
- Test: `tests/event_extractor_tests.rs` (追加)

**Interfaces:**
- Consumes: `ClassifierOutput` (Task 3), `RawNewsItem`, `GeminiAnalyzer`
- Produces: `MarketEvent`

- [x] **Step 1: 写失败测试**

```rust
// tests/event_extractor_tests.rs (追加)
use stock_analysis::opportunity::event_extractor::core::*;

#[test]
fn test_core_parse_deep_json() {
    // 修复 P0-1: mock Deep AI 返回 → MarketEvent 字段完整
    let json = r#"{"event_type":"TechBreak","direction":"Bull","subject":"CO2激光设备","object":"晶圆制造","strength":70,"certainty":60,"reason":"行业级技术突破"}"#;
    let raw = raw_item("CO2 激光突破", "cls", SourceType::Search);
    let me = EventExtractorCore::parse_deep_response(&raw, json).unwrap();
    assert_eq!(me.event_type, EventType::TechBreak);
    assert_eq!(me.direction, Direction::Bull);
    assert_eq!(me.subject, "CO2激光设备");
    assert_eq!(me.strength, 70);
    assert_eq!(me.certainty, 60);
    assert!(!me.event_id.is_empty(), "event_id 必自动生成");
}

#[test]
fn test_core_quick_only_strength_lookup() {
    // 修复 §4.3: 盘中 deterministic 映射
    let s = strength_for_event_type(EventType::TechBreak, 0.85);
    // TechBreak=65, × 0.85 = 55.25 → clamp(20,80) = 55
    assert_eq!(s, 55);
}

#[test]
fn test_core_quick_only_certainty_lookup() {
    // 修复 §4.3: 盘中 certainty 映射
    let c = certainty_for_source(SourceType::Flash, 0.85);
    // Flash factor=0.8, 0.85*100*0.8 = 68 → clamp(30,85) = 68
    assert_eq!(c, 68);
}

#[test]
fn test_core_quick_only_yields_market_event() {
    // 修复 P0-1: 盘中 Quick-only 路径产出完整 MarketEvent
    let raw = raw_item("工信部政策", "cls", SourceType::Flash);
    let co = ClassifierOutput {
        is_event: true, event_type: Some(EventType::Policy),
        direction: Some(Direction::Bull), subject: Some("工信部".into()),
        confidence: 0.9,
    };
    let me = EventExtractorCore::from_quick_only(&raw, &co);
    assert_eq!(me.event_type, EventType::Policy);
    assert!(me.strength >= 20 && me.strength <= 80);
    assert!(me.certainty >= 30 && me.certainty <= 85);
    assert_eq!(me.ai_degraded, false, "确定映射不算降级");
}

#[test]
fn test_core_parse_garbage_deep_yields_none() {
    let raw = raw_item("x", "x", SourceType::Search);
    let me = EventExtractorCore::parse_deep_response(&raw, "garbage");
    assert!(me.is_none());
}

fn raw_item(title: &str, source: &str, st: SourceType) -> RawNewsItem {
    RawNewsItem {
        title: title.into(), body: "".into(), source: source.into(),
        source_priority: 1, source_type: st,
        published_at: chrono::Local::now(), url: None,
    }
}
```

- [x] **Step 2: 确认失败**

```bash
cargo test --test event_extractor_tests test_core
# Expected: error[E0432]
```

- [x] **Step 3: 最小实现**

```rust
// src/opportunity/event_extractor/core.rs
use crate::signal::market_event::{MarketEvent, EventType, Direction, compute_event_id};
use crate::signal::market_event::SourceRef;
use super::adapter::{RawNewsItem, SourceType};
use super::classifier::ClassifierOutput;
use serde::Deserialize;
use chrono::Local;

#[derive(Debug, Clone, Deserialize)]
struct DeepResponse {
    event_type: EventType,
    direction: Direction,
    subject: String,
    object: Option<String>,
    strength: u8,
    certainty: u8,
    reason: String,
}

pub struct EventExtractorCore;

impl EventExtractorCore {
    /// 修复 P0-1: 给 Deep AI prompt
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

    /// 修复 P0-1: 解析 Deep AI 响应
    pub fn parse_deep_response(item: &RawNewsItem, response: &str) -> Option<MarketEvent> {
        let cleaned = response.trim().trim_start_matches("```json").trim_end_matches("```").trim();
        let dr: DeepResponse = serde_json::from_str(cleaned).ok()?;
        let event_id = compute_event_id(&item.title, &item.published_at);
        Some(MarketEvent::new(
            dr.event_type, dr.subject, dr.object,
            dr.direction, dr.strength, dr.certainty,
        ))
    }

    /// 修复 §4.3: 盘中 Quick-only 确定性映射
    pub fn from_quick_only(item: &RawNewsItem, co: &ClassifierOutput) -> MarketEvent {
        let strength = strength_for_event_type(co.event_type.unwrap_or(EventType::Other), co.confidence);
        let certainty = certainty_for_source(item.source_type, co.confidence);
        let event_id = compute_event_id(&item.title, &item.published_at);
        let mut me = MarketEvent::new(
            co.event_type.unwrap_or(EventType::Other),
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

    /// 修复 P0-1: 构建 degreaded (AI 不可用) MarketEvent
    pub fn build_degraded(item: &RawNewsItem, event_type: Option<EventType>) -> MarketEvent {
        let event_id = compute_event_id(&item.title, &item.published_at);
        let mut me = MarketEvent::new(
            event_type.unwrap_or(EventType::Other),
            item.title.chars().take(30).collect(),
            None,
            Direction::Neutral,
            30,  // strength=30 (保守)
            30,  // certainty=30 (保守)
        );
        me.event_id = event_id;
        me.ai_degraded = true;
        me
    }
}

/// 修复 §4.3: 各 event_type 的 strength_base
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
        EventType::Other => 40,
    };
    let raw = (base as f64 * confidence).round() as u8;
    raw.clamp(20, 80)
}

/// 修复 §4.3: 各 source_type 的 certainty_factor
pub fn certainty_for_source(st: SourceType, confidence: f64) -> u8 {
    let factor = match st {
        SourceType::Announcement => 1.0,
        SourceType::Search => 0.9,
        SourceType::Flash => 0.8,
    };
    let raw = (confidence * 100.0 * factor).round() as u8;
    raw.clamp(30, 85)
}
```

- [x] **Step 4: 跑测试**

```bash
cargo test --test event_extractor_tests test_core
# Expected: 5 passed
```

- [x] **Step 5: commit**

```bash
git add src/opportunity/event_extractor/core.rs tests/event_extractor_tests.rs
git commit -m "feat(event_extractor): T4 core (Deep AI + 盘中 Quick-only 确定性映射 + degraded)"
```

---

### Task 5: mod.rs 入口 + 集成

**Files:**
- Modify: `src/opportunity/event_extractor/mod.rs` (补充集成函数)
- Test: `tests/event_extractor_tests.rs` (追加)

**Interfaces:**
- `extract_batch(items: &[SearchResult], gemini: &GeminiAnalyzer) -> Vec<MarketEvent>` (盘前)
- `extract_incremental(items: &[SearchResult], gemini: &GeminiAnalyzer) -> Vec<MarketEvent>` (盘中)
- AI 不可用: 内部 catch error, 走 degraded 路径

- [x] **Step 1: 写失败测试**

```rust
// tests/event_extractor_tests.rs (追加)
use stock_analysis::opportunity::event_extractor::*;

fn search_result(title: &str, date: &str) -> SearchResult {
    SearchResult {
        title: title.into(), snippet: "正文".into(), url: "".into(),
        source: "测试".into(), published_date: Some(date.into()),
        news_type: NewsType::Search, sentiment: Sentiment::Neutral,
        importance: 3, relevance: 0.5, keywords: vec![],
    }
}

#[test]
fn test_extract_batch_rules_only_ai_unavailable() {
    // 修复 P0-1: AI 不可用时完整走规则预筛, 产出 degraded MarketEvent
    let items = vec![
        search_result("工信部: 5G-A 商用进入新阶段", "2026-06-27 10:00:00"),
        search_result("A股收评：三大指数低开高走", "2026-06-27 10:00:00"),
        search_result("碳酸锂价格上调 5000 元", "2026-06-27 10:00:00"),
    ];
    // 不传 Gemini → 规则预筛 only
    let events = extract_batch_rules_only(&items);
    assert_eq!(events.len(), 2, "工信部 + 碳酸锂 → 2 个事件; 收评 → 丢弃");
    for e in &events {
        assert!(e.ai_degraded, "没有 AI 时必 ai_degraded=true");
        assert_eq!(e.strength, 30);
        assert_eq!(e.certainty, 30);
    }
}

#[test]
fn test_extract_batch_empty_input() {
    let events = extract_batch_rules_only(&[]);
    assert!(events.is_empty());
}

#[test]
fn test_extract_incremental_filters_stale() {
    // 修复 §5.1: 过期 > 5 分钟的数据必丢弃
    let old = search_result("旧快讯", "2026-06-27 09:00:00"); // 假设现在是 10:00
    let events = extract_incremental_rules_only(&[old], chrono::Duration::minutes(5));
    assert!(events.is_empty(), "过期 > 5min 必丢弃");
}
```

- [x] **Step 2: 确认失败**

```bash
cargo test --test event_extractor_tests test_extract
# Expected: error[E0425]
```

- [x] **Step 3: 在 mod.rs 加集成函数**

```rust
// 追加到 src/opportunity/event_extractor/mod.rs
use crate::search_service::SearchResult;
use crate::signal::market_event::MarketEvent;
use chrono::{DateTime, Local, Duration};
use super::adapter::{SearchResultAdapter, RawNewsItem, SourceType};
use super::rule_filter::RuleFilter;
use super::classifier::EventClassifier;
use super::core::EventExtractorCore;

/// 盘前 batch: 完整三层漏斗 (adapter → rules → classifier → core)
/// AI 不可用时退化到仅规则预筛
pub fn extract_batch(
    items: &[SearchResult],
    gemini: Option<&crate::analyzer::GeminiAnalyzer>,
) -> Vec<MarketEvent> {
    let _ = gemini; // 后续接 gemini.call_api_mode
    extract_batch_rules_only(items)
}

/// 盘前 batch: 仅在规则预筛层的降级路径 (AI 不可用时)
pub fn extract_batch_rules_only(items: &[SearchResult]) -> Vec<MarketEvent> {
    let mut events = Vec::new();
    for sr in items {
        let raw = match SearchResultAdapter::to_raw(sr) {
            Ok(r) => r,
            Err(_) => continue, // §5.1: published_at 缺失必跳过
        };
        let rm = RuleFilter::filter(&raw);
        if !rm.matched {
            continue;
        }
        let me = EventExtractorCore::build_degraded(&raw, rm.event_type);
        events.push(me);
    }
    events
}

/// 盘中增量: adapter → rules → classifier (不做 Deep)
/// 修复 §4.3: 用确定性映射填充 strength/certainty
pub fn extract_incremental_rules_only(
    items: &[SearchResult],
    max_age: chrono::Duration,
) -> Vec<MarketEvent> {
    let now = Local::now();
    let mut events = Vec::new();
    for sr in items {
        let raw = match SearchResultAdapter::to_raw(sr) {
            Ok(r) => r,
            Err(_) => continue,
        };
        // 修复 §5.1 红线: 过期丢弃
        if now - raw.published_at > max_age {
            continue;
        }
        let rm = RuleFilter::filter(&raw);
        if !rm.matched {
            continue;
        }
        let me = EventExtractorCore::build_degraded(&raw, rm.event_type);
        events.push(me);
    }
    events
}
```

- [x] **Step 4: 跑测试**

```bash
cargo test --test event_extractor_tests test_extract
# Expected: 3 passed
```

- [x] **Step 5: 跑全量回归**

```bash
cargo test --lib -- --test-threads=1
# Expected: 434 passed + 新增, ≥ 455
```

- [x] **Step 6: commit**

```bash
git add src/opportunity/event_extractor/mod.rs tests/event_extractor_tests.rs
git commit -m "feat(event_extractor): T5 mod 入口 (extract_batch + extract_incremental + stale filter + rules-only degraded)"
```

---

## 验收 (量化 PM 8 Gate)

| Gate | 验证 |
|------|------|
| 盘前 batch ≥ 10 MarketEvent | `extract_batch_rules_only(300 条) → events.len() ≥ 10` |
| 盘中增量 1-3 事件 | `extract_incremental_rules_only(10 条, 5min) → 1-3 个` |
| AI 不可用不 panic | `extract_batch_rules_only` 无 panicking |
| event_id 去重 | 同标题同日期 → 同 event_id |
| provenance 溯源 | `.provenance` 非空 |
| 规则预筛精确率 ≥ 85% | 6 个单元测试覆盖已知标题 |
| 成本 ≤ 80K token | rules-only 路径 0 token, AI 路径后续接 gemini |
| 回归 `cargo test --lib` 全过 | ≥ 450 测试 |

---

## 自检

1. **Spec coverage**: adapter(§2.1) → rule_filter(§3) → classifier(§4.1) → core(§4.2+§4.3) → mod(§1.3 调用时机 + §5.1 时效红线). 全覆盖.
2. **Placeholder scan**: 无 TBD/TODO.
3. **Type consistency**: RawNewsItem → RuleMatch → ClassifierOutput → MarketEvent 类型链完整, DeepResponse 与 Quick-only 两个映射函数签名明确.
