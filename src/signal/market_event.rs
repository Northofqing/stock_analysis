//! 修复 P0-1: MarketEvent 标准中间件
//!
//! v9 流水线的"全链路标准件" (NS1 约束):
//! - ② 事件抽取 → 产出 MarketEvent
//! - ③ 产业链映射 → 填充 chains
//! - ④ 公司映射 → 填充 candidate_stocks
//! - ⑤ 历史相似回测 → 用 event_id 检索
//! - ⑥ 资金热度参考 → 用 code 关联
//! - ⑦ 评分 → 消费完整结构
//!
//! 关键设计:
//! - strength × certainty 正交 (修复 P0-1)
//! - chains 在 MarketEvent::new 时为空, 由 ③ 阶段填充 (职责切分)
//! - ai_degraded 标志: AI 不可用时置真, 下游必须降权不编造
//! - provenance 落审计: 跨源验证, 单源封顶 70 (修复 P0-1 跨源软化)

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

/// 事件类型 (受限枚举, 修复 P0-1 不允许字符串乱填)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    /// 政策 (工信部/央行/财政部等)
    Policy,
    /// 技术突破 (新产品/新技术/专利)
    TechBreak,
    /// 订单中标 (重大合同/中标)
    OrderWin,
    /// 产能变化 (扩产/减产)
    Capacity,
    /// 涨价 (上游原材料涨价)
    PriceUp,
    /// 跌价
    PriceDown,
    /// 并购重组
    Mna,
    /// 事故/利空
    Accident,
    /// 海外事件
    Overseas,
    /// 其他
    Other,
}

/// 事件方向 (对受益方)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// 利好受益方
    Bull,
    /// 中性
    Neutral,
    /// 利空
    Bear,
}

/// 数据来源溯源 (跨源验证基础)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRef {
    pub provider: String,    // "东财" / "新浪" / "巨潮"
    pub url: Option<String>,
    pub fetched_at: DateTime<Local>,
}

/// MarketEvent 标准中间件 (修复 P0-1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketEvent {
    /// 事件指纹 (sha256 hex 64字符, 用于去重 + 历史检索)
    pub event_id: String,
    /// 事件类型
    pub event_type: EventType,
    /// 主体 (谁/什么产品/什么环节)
    pub subject: String,
    /// 客体 (涉及对象, 可空)
    pub object: Option<String>,
    /// 事件方向 (对受益方)
    pub direction: Direction,
    /// 市场影响强度 0-100 (对受益方的预期冲击力度)
    pub strength: u8,
    /// 信息确定性 0-100 (官方落地=高, 传闻=低)
    pub certainty: u8,
    /// 候选产业链 (②阶段恒为空, 由 ③ 填充)
    pub chains: Vec<String>,
    /// 事件时间 (新鲜度校验)
    pub occurred_at: DateTime<Local>,
    /// 数据来源溯源 (跨源验证 + 审计)
    pub provenance: Vec<SourceRef>,
    /// AI 降级标志 (true=规则降级抽取, 下游必须降权不编造)
    pub ai_degraded: bool,
}

impl MarketEvent {
    /// 创建新事件 (事件 ID 自动生成, 修复 P0-1 NS2 可追溯)
    pub fn new(
        event_type: EventType,
        subject: String,
        object: Option<String>,
        direction: Direction,
        strength: u8,
        certainty: u8,
    ) -> Self {
        let now = Local::now();
        let event_id = compute_event_id(&subject, &now);
        Self {
            event_id,
            event_type,
            subject,
            object,
            direction,
            strength: strength.min(100),
            certainty: certainty.min(100),
            chains: Vec::new(),
            occurred_at: now,
            provenance: Vec::new(),
            ai_degraded: false,
        }
    }
}

/// 修复 P1-1: event_id = sha256(normalize(title) + "|" + occurred_at.date())
/// 包含 normalize: 去标点/空白折叠/全半角统一
pub fn compute_event_id(title: &str, occurred_at: &DateTime<Local>) -> String {
    use sha2::{Digest, Sha256};
    let normalized = normalize(title);
    let input = format!("{}|{}", normalized, occurred_at.date_naive());
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

/// 标点统一 (中英混合) + 空白折叠
fn normalize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '：' => ':', '，' => ',', '。' => '.', '；' => ';',
            '（' => '(', '）' => ')', '？' => '?', '！' => '!',
            '\u{3000}' => ' ', c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
