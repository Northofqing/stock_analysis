//! BR-059/BR-105/BR-108/BR-137/BR-138/BR-140/BR-168 announcement projection.
//!
//! Acquisition belongs to [`crate::data_gateway::EventCalendarGateway`].
//! This module retains only downstream title classification and an
//! evidence-preserving projection of the fields that CNInfo actually supplies.

use anyhow::{anyhow, Result};

use crate::config::AnnounceKeywordsFile;
use crate::data_gateway::{BatchEvidence, EventAnnouncement, GatewayBatch};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnnLevel {
    Emergency,
    Important,
    Info,
    Skip,
}

#[derive(Debug, Clone)]
pub struct Announcement {
    pub code: String,
    pub name: String,
    pub title: String,
    pub date: String,
    pub summary: String,
    pub content: String,
    pub level: AnnLevel,
    pub reason: String,
    pub external_id: Option<String>,
    pub url: Option<String>,
}

impl Announcement {
    pub fn published_on(&self) -> Result<chrono::NaiveDate> {
        parse_notice_date(&self.date)
    }
}

fn parse_notice_date(raw: &str) -> Result<chrono::NaiveDate> {
    let raw = raw.trim();
    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Ok(timestamp.date_naive());
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Ok(date);
    }
    if let Ok(timestamp) = chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S") {
        return Ok(timestamp.date());
    }
    Err(anyhow!("公告 provider published_at 非法: {raw}"))
}

/// Downstream announcement view plus the immutable Gateway batch evidence.
#[derive(Debug, Clone)]
pub struct AnnouncementBatch {
    pub announcements: Vec<Announcement>,
    pub evidence: BatchEvidence,
}

pub fn announcement_title_is_immediately_actionable(title: &str) -> bool {
    let creditor_procedure = title.contains("通知债权人")
        && (title.contains("减少注册资本") || (title.contains("注销") && title.contains("回购")));
    let reduction_completed = title.contains("减持")
        && ["期限届满", "时间届满", "实施完毕", "实施完成"]
            .iter()
            .any(|marker| title.contains(marker));
    !creditor_procedure && !reduction_completed
}

pub fn announcement_is_immediate_notification_candidate(announcement: &Announcement) -> bool {
    !matches!(announcement.level, AnnLevel::Skip)
        && announcement_title_is_immediately_actionable(&announcement.title)
}

/// BR-168 maps one complete EventCalendar batch into the existing analysis
/// model. Optional fields that CNInfo does not provide stay empty.
pub fn project_event_calendar_batch(
    batch: GatewayBatch<EventAnnouncement>,
    keywords: &AnnounceKeywordsFile,
) -> Result<AnnouncementBatch> {
    let evidence = batch.evidence().clone();
    let announcements = batch
        .records()
        .iter()
        .map(|event| {
            let (level, reason) = classify_title(&event.title, keywords);
            Announcement {
                code: event.code.clone(),
                name: String::new(),
                title: event.title.clone(),
                date: event.published_at.clone(),
                summary: String::new(),
                content: String::new(),
                level,
                reason,
                external_id: Some(event.announcement_id.clone()),
                url: Some(event.canonical_url.clone()),
            }
        })
        .collect();
    Ok(AnnouncementBatch {
        announcements,
        evidence,
    })
}

fn classify_title(title: &str, keywords: &AnnounceKeywordsFile) -> (AnnLevel, String) {
    if let Some(keyword) = keywords
        .emergency
        .iter()
        .find(|keyword| title.contains(keyword.as_str()))
    {
        return (AnnLevel::Emergency, format!("标题含'{keyword}'，直接告警"));
    }

    let reduction_below_one_percent =
        title.contains("减持") && extract_reduction_pct(title).is_some_and(|value| value < 1.0);
    if let Some(keyword) = keywords.important.iter().find(|keyword| {
        !(reduction_below_one_percent && keyword.as_str() == "减持")
            && title.contains(keyword.as_str())
    }) {
        return (AnnLevel::Important, format!("标题含'{keyword}'"));
    }
    if let Some(keyword) = keywords
        .positive
        .iter()
        .find(|keyword| title.contains(keyword.as_str()))
    {
        return (AnnLevel::Info, format!("利好: '{keyword}'"));
    }
    (AnnLevel::Skip, String::new())
}

fn extract_reduction_pct(title: &str) -> Option<f64> {
    title
        .split(|character: char| !character.is_ascii_digit() && character != '.')
        .filter(|part| !part.is_empty())
        .find_map(|part| {
            let suffix = title.find(part)?.checked_add(part.len())?;
            title[suffix..]
                .trim_start()
                .starts_with('%')
                .then(|| part.parse::<f64>().ok())
                .flatten()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_domain::ProviderId;

    fn keywords() -> AnnounceKeywordsFile {
        AnnounceKeywordsFile {
            emergency: vec!["立案调查".into()],
            important: vec!["减持".into(), "监管函".into()],
            positive: vec!["回购".into()],
        }
    }

    #[test]
    fn publication_date_rejects_trailing_input() {
        let announcement = Announcement {
            code: "TEST_CODE_600519".into(),
            name: "test".into(),
            title: "test".into(),
            date: "2026-07-25 00:00:00.000".into(),
            summary: String::new(),
            content: String::new(),
            level: AnnLevel::Info,
            reason: String::new(),
            external_id: None,
            url: None,
        };
        assert!(announcement.published_on().is_err());
    }

    #[test]
    fn cninfo_projection_preserves_source_fields_and_missing_detail() {
        let batch = GatewayBatch::Available {
            records: vec![EventAnnouncement {
                announcement_id: "TEST_CODE_ANN".into(),
                code: "TEST_CODE_600519".into(),
                category: Some("回购".into()),
                title: "关于回购股份的公告".into(),
                published_at: "2026-07-25T20:00:00+08:00".into(),
                canonical_url: "https://example.invalid/TEST_CODE_ANN".into(),
            }],
            evidence: BatchEvidence {
                provider: ProviderId::Cninfo,
                source: "TEST_CODE_CNINFO".into(),
                source_at: Some("2026-07-25T20:00:00+08:00".into()),
                observed_at: "2026-07-25T20:00:01+08:00".into(),
                batch_id: "TEST_CODE_BATCH".into(),
            },
        };
        let projected =
            project_event_calendar_batch(batch, &keywords()).expect("complete projection");
        let announcement = &projected.announcements[0];
        assert_eq!(announcement.level, AnnLevel::Info);
        assert_eq!(announcement.external_id.as_deref(), Some("TEST_CODE_ANN"));
        assert!(announcement.name.is_empty());
        assert!(announcement.summary.is_empty());
        assert!(announcement.content.is_empty());
        assert_eq!(
            announcement.published_on().unwrap(),
            chrono::NaiveDate::from_ymd_opt(2026, 7, 25).unwrap()
        );
    }

    #[test]
    fn sub_one_percent_reduction_does_not_match_generic_reduction_keyword() {
        assert_eq!(
            classify_title("股东计划减持不超过0.5%的公告", &keywords()).0,
            AnnLevel::Skip
        );
    }
}
