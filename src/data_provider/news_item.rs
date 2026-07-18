//! Registered business rules: BR-066.
//! 新闻条目结构 + SHA256 content_hash helper (review #16)
//!
//! 设计动机: news_dedup 表只保留 5min 滑窗, 适合"短时间内不要再推"用途;
//! 但新闻内容详存 + 跨重启追溯 + 后续 LLM 复盘, 需要永久保存.
//! `news_items` 表存原始条目; `content_hash` 用于重复检测 (同一 title+summary 不重复入库).
//!
//! 当前 task (Task 9) 只定义结构 + 迁移 + insert helper.
//! 真正的 fetch (sina_financial / sina_stock) 在后续 task 接入.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

/// 新闻条目 (sina_financial / sina_stock / 后续其他源)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NewsItem {
    /// 数据源标识: "sina_financial" | "sina_stock"
    pub source: String,
    /// 外部 ID (目前用 url 作为 ID; 后续可换真实 ID)
    pub external_id: String,
    /// 分类: "财经要闻" | "个股新闻"
    pub category: String,
    /// 6 位股票代码 (仅个股新闻; 财经要闻为 None)
    pub code: Option<String>,
    /// 标题
    pub title: String,
    /// 摘要 (来自 sina_rss 描述字段)
    pub summary: String,
    /// 原文 url
    pub url: String,
    /// 来源展示名 (如 "新浪财经")
    pub source_name: String,
    /// 原始发布时间
    pub published_at: DateTime<Utc>,
    /// 抓取时间 (本地入库时间)
    pub fetched_at: DateTime<Utc>,
    /// `content_hash(title, summary)` — dedup key (同源同 ID 但内容变时可检测)
    pub content_hash: String,
}

/// SHA256 hex of (title + summary) — 用于 dedup.
///
/// 输出小写 64 字符十六进制 (标准 SHA256 输出).
/// 不带分隔符: title 与 summary 直接字节拼接, 边界由调用方控制 (本项目内 title/summary 不含空字节).
pub fn content_hash(title: &str, summary: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    hasher.update(summary.as_bytes());
    format!("{:x}", hasher.finalize())
}
