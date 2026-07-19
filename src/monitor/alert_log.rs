//! Registered business rules: BR-045.
//! 告警本地归档：每条告警落盘为 JSONL + Markdown 双格式。
//! 路径：reports/alerts/{date}.jsonl  +  reports/alerts/{date}.md

use crate::monitor::detector::AlertEvent;
use chrono::Local;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn alerts_dir() -> PathBuf {
    PathBuf::from("reports/alerts")
}

fn today_file(ext: &str) -> PathBuf {
    let date = Local::now().format("%Y%m%d").to_string();
    alerts_dir().join(format!("{}.{}", date, ext))
}

fn write_jsonl(mut writer: impl Write, event: &AlertEvent) -> std::io::Result<()> {
    serde_json::to_writer(&mut writer, &AlertRecord::from(event)).map_err(std::io::Error::other)?;
    writeln!(writer)
}

fn write_markdown(mut writer: impl Write, event: &AlertEvent) -> std::io::Result<()> {
    use crate::monitor::alert::format_alert;
    writeln!(writer, "---\n{}\n", format_alert(event))
}

/// 追加一条告警到 JSONL（机器可读）。打开、序列化或写入失败均显式返回。
pub fn append_jsonl(event: &AlertEvent) -> std::io::Result<()> {
    let dir = alerts_dir();
    fs::create_dir_all(&dir)?;
    let path = today_file("jsonl");
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    write_jsonl(file, event)
}

/// 追加一条告警到 Markdown（人可读）。打开或写入失败均显式返回。
pub fn append_md(event: &AlertEvent) -> std::io::Result<()> {
    let dir = alerts_dir();
    fs::create_dir_all(&dir)?;
    let path = today_file("md");
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    write_markdown(file, event)
}

/// 批量追加（一次写入减少 IO）
pub fn append_batch(events: &[AlertEvent]) -> std::io::Result<()> {
    for e in events {
        append_jsonl(e)?;
        append_md(e)?;
    }
    Ok(())
}

/// 读取今日告警
pub fn read_today() -> Vec<String> {
    let path = today_file("md");
    match fs::read_to_string(&path) {
        Ok(s) => s
            .split("---\n")
            .filter(|p| !p.trim().is_empty())
            .map(|p| p.trim().to_string())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// 今日告警统计
pub fn today_stats() -> (usize, usize, usize) {
    let alerts = read_today();
    let emergency = alerts.iter().filter(|a| a.contains("🔴")).count();
    let important = alerts.iter().filter(|a| a.contains("🟠")).count();
    let info = alerts.iter().filter(|a| a.contains("🟡")).count();
    (emergency, important, info)
}

// ── JSON 记录 ──

#[derive(serde::Serialize)]
struct AlertRecord {
    triggered_at: String,
    code: String,
    name: String,
    level: String,
    category: String,
    message: String,
    price: Option<f64>,
    change_pct: Option<f64>,
    main_flow_yi: Option<f64>,
    news_title: Option<String>,
    news_importance: Option<u8>,
    attribution_decision: Option<String>,
    routed_external_id: Option<String>,
    t1_locked: bool,
}

impl AlertRecord {
    fn from(e: &AlertEvent) -> Self {
        AlertRecord {
            triggered_at: e.triggered_at.to_rfc3339(),
            code: e.code.clone(),
            name: e.name.clone(),
            level: e.level.label().to_string(),
            category: e.category.label().to_string(),
            message: e.message.clone(),
            price: e.detail.price,
            change_pct: e.detail.change_pct,
            main_flow_yi: e.detail.main_flow_yi,
            news_title: e.detail.news_title.clone(),
            news_importance: e.detail.news_importance,
            attribution_decision: e.detail.ai_decision.clone(),
            routed_external_id: e.routed_external_id.clone(),
            t1_locked: e.detail.t1_locked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::detector::{AlertCategory, AlertDetail, AlertLevel};

    fn e() -> AlertEvent {
        AlertEvent {
            level: AlertLevel::Important,
            category: AlertCategory::MainOutflow,
            code: "TEST_CODE_000001".into(),
            name: "测试".into(),
            message: "测试告警".into(),
            detail: AlertDetail {
                price: Some(10.0),
                change_pct: Some(-3.0),
                volume_ratio: None,
                main_flow_yi: Some(-0.5),
                threshold: None,
                news_title: None,
                news_summary: None,
                news_importance: None,
                ai_decision: None,
                t1_locked: false,
                extra: None,
            },
            triggered_at: Local::now(),
            routed_external_id: None,
        }
    }

    #[test]
    fn test_append_and_read() {
        let event = e();
        append_md(&event).unwrap();
        let alerts = read_today();
        assert!(!alerts.is_empty());
    }

    #[test]
    fn test_append_jsonl() {
        append_jsonl(&e()).unwrap();
        let path = today_file("jsonl");
        assert!(path.exists());
    }

    #[test]
    fn jsonl_contains_structured_attribution_evidence() {
        let mut event = e();
        event.detail.news_title = Some("TEST_CODE 快讯".into());
        event.detail.news_importance = Some(4);
        event.detail.ai_decision = Some("产业链催化 | 置信度B".into());
        let mut output = Vec::new();

        write_jsonl(&mut output, &event).unwrap();

        let record: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(record["news_importance"], 4);
        assert_eq!(record["attribution_decision"], "产业链催化 | 置信度B");
        assert!(record["triggered_at"].as_str().unwrap().contains('T'));
    }

    #[test]
    fn writer_failure_is_returned() {
        struct FailingWriter;
        impl Write for FailingWriter {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("TEST_CODE forced failure"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        assert!(write_jsonl(FailingWriter, &e()).is_err());
    }

    #[test]
    fn test_today_stats() {
        append_md(&e()).unwrap();
        let (em, im, inf) = today_stats();
        assert!(em + im + inf > 0);
    }
}
