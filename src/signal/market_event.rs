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
    /// 事件指纹 (sha256 hex 64字符, 用于精确去重 + 历史检索)
    pub event_id: String,
    /// 修复 P1-1: SimHash 64-bit, 用于跨源模糊去重
    /// 财联社 vs 新浪 同事件不同标题 → event_id 不同但 simhash 接近
    /// 24h 内汉明距离 ≤ 3 → 同一事件
    #[serde(default)]
    pub simhash: u64,
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
    /// 修复 P0-2: 过期事件标记 (true=超过 max_age, 不参与评分, 入审计)
    /// 真实含义: published_at 距 now 超过 batch/incremental 阈值
    /// 用途: 调用方按 stale 分桶, fresh 入评分, stale 入审计 (避免误推旧闻)
    #[serde(default)]
    pub stale: bool,
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
        let simhash = compute_simhash(&subject, "");
        Self {
            event_id,
            simhash,
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
            stale: false,
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

/// 中文常见停用词 / 助词 / 功能字 (量化 PM 视角: 这些字符组成的 bigram 是噪声)
/// 例: "的了" "是在" "和中" 都是无信号 token, 占 bit 反而稀释真信号
const STOP_CHARS: &[char] = &[
    '的', '了', '在', '是', '和', '等', '与', '为', '于', '及',
    '或', '有', '其', '之', '也', '就', '都', '还', '把', '被',
    '要', '能', '会', '可', '过', '又', '再', '这', '那', '此',
    '某', '中', '上', '下', '一', '个', '不', '但',
];

/// 修复 P1-1: 噪声 token 过滤
/// 两个字符都是停用词 → 跳过, 不参与 simhash 累计
/// 修复: 中文 bigram "工信"+"信部" 不会被 "的了" 这类噪声稀释
fn is_noise_token(token: &str) -> bool {
    let mut chars = token.chars();
    match (chars.next(), chars.next()) {
        (Some(a), Some(b)) => STOP_CHARS.contains(&a) && STOP_CHARS.contains(&b),
        _ => false,
    }
}

/// 修复 P1-1: SimHash 64-bit, 用于跨源模糊去重
/// tokenize: 字符 bigram, 跳过空白/ASCII 标点 + 停用词组合
/// 64-bit hash: 每个 token 的稳定 hash 的 bit 累加 (sha256 截前 8 字节)
pub fn compute_simhash(title: &str, body: &str) -> u64 {
    let combined = format!("{} {}", normalize(title), normalize(body));
    // 修复: 过滤空白和 ASCII 标点, 避免 "部 " " 5" 这类无意义 bigram
    let chars: Vec<char> = combined
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation())
        .collect();
    if chars.len() < 2 {
        return 0;
    }
    let mut v: [i32; 64] = [0; 64];
    for window in chars.windows(2) {
        let token: String = window.iter().collect();
        if is_noise_token(&token) { continue; }
        let token_hash = stable_hash_token(&token);
        for bit in 0..64 {
            if (token_hash >> bit) & 1 == 1 {
                v[bit] += 1;
            } else {
                v[bit] -= 1;
            }
        }
    }
    let mut result: u64 = 0;
    for (i, &count) in v.iter().enumerate() {
        if count > 0 {
            result |= 1u64 << i;
        }
    }
    result
}

/// 修复 P1-1: 跨进程稳定 token hash (sha256 截前 8 字节 → u64)
/// 之前用 std DefaultHasher, 每次进程启动 seed 不同 → shadow 阶段索引在 gray 阶段失效
/// sha256 是确定性算法, 同输入永远同输出, 跨进程/跨 OS/跨 Rust 版本都稳定
fn stable_hash_token(s: &str) -> u64 {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let result = hasher.finalize();
    u64::from_le_bytes([
        result[0], result[1], result[2], result[3],
        result[4], result[5], result[6], result[7],
    ])
}

/// 修复 P1-1: SimHash 汉明距离 (量化"两事件多相似")
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}
