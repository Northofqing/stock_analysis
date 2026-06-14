//! 告警本地归档：每条告警落盘为 JSONL + Markdown 双格式。
//! 路径：reports/alerts/{date}.jsonl  +  reports/alerts/{date}.md

use crate::monitor::detector::AlertEvent;
use chrono::Local;
use log::warn;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

fn alerts_dir() -> PathBuf {
    let dir = PathBuf::from("reports/alerts");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn today_file(ext: &str) -> PathBuf {
    let date = Local::now().format("%Y%m%d").to_string();
    alerts_dir().join(format!("{}.{}", date, ext))
}

/// 追加一条告警到 JSONL（机器可读）
pub fn append_jsonl(event: &AlertEvent) {
    let line = serde_json::to_string(&AlertRecord::from(event)).unwrap_or_default();
    let path = today_file("jsonl");
    let mut f = match fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(e) => { warn!("[AlertLog] 无法打开 {}: {}", path.display(), e); return; }
    };
    let _ = writeln!(f, "{}", line);
}

/// 追加一条告警到 Markdown（人可读）
pub fn append_md(event: &AlertEvent) {
    use crate::monitor::alert::format_alert;
    let text = format_alert(event);
    let path = today_file("md");
    let mut f = match fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(e) => { warn!("[AlertLog] 无法打开 {}: {}", path.display(), e); return; }
    };
    let _ = writeln!(f, "---\n{}\n", text);
}

/// 批量追加（一次写入减少 IO）
pub fn append_batch(events: &[AlertEvent]) {
    for e in events {
        append_jsonl(e);
        append_md(e);
    }
}

/// 读取今日告警
pub fn read_today() -> Vec<String> {
    let path = today_file("md");
    match fs::read_to_string(&path) {
        Ok(s) => s.split("---\n").filter(|p| !p.trim().is_empty()).map(|p| p.trim().to_string()).collect(),
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
    time: String,
    code: String,
    name: String,
    level: String,
    category: String,
    message: String,
    price: Option<f64>,
    change_pct: Option<f64>,
    main_flow_yi: Option<f64>,
    t1_locked: bool,
}

impl AlertRecord {
    fn from(e: &AlertEvent) -> Self {
        AlertRecord {
            time: e.triggered_at.format("%H:%M:%S").to_string(),
            code: e.code.clone(),
            name: e.name.clone(),
            level: e.level.label().to_string(),
            category: e.category.label().to_string(),
            message: e.message.clone(),
            price: e.detail.price,
            change_pct: e.detail.change_pct,
            main_flow_yi: e.detail.main_flow_yi,
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
            code: "000001".into(), name: "测试".into(),
            message: "测试告警".into(),
            detail: AlertDetail {
                price: Some(10.0), change_pct: Some(-3.0),
                volume_ratio: None, main_flow_yi: Some(-0.5),
                threshold: None, news_title: None,
                t1_locked: false, extra: None,
            },
            triggered_at: Local::now(),
        }
    }

    #[test]
    fn test_append_and_read() {
        let event = e();
        append_md(&event);
        let alerts = read_today();
        assert!(!alerts.is_empty());
    }

    #[test]
    fn test_append_jsonl() {
        append_jsonl(&e());
        let path = today_file("jsonl");
        assert!(path.exists());
    }

    #[test]
    fn test_today_stats() {
        let (em, im, inf) = today_stats();
        // At least one important alert from previous test
        assert!(em + im + inf > 0);
    }
}
