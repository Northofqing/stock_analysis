use chrono::{DateTime, Local, NaiveDateTime};
use crate::search_service::SearchResult;

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
        let date_str = sr.published_date.as_deref()
            .ok_or_else(|| format!("E_INVALID_PUBLISHED_AT: published_date 缺失 (source={}, title={})",
                sr.source, sr.title))?;
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
