//! Registered business rules: BR-045.
//! 告警本地归档：每条告警落盘为 JSONL + Markdown 双格式。
//! 路径：reports/alerts/{date}.jsonl  +  reports/alerts/{date}.md

use crate::monitor::detector::AlertEvent;
use crate::risk::env_guard::{current_env, is_test_code, runtime_is_test_process, TradingEnv};
use chrono::Local;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn alerts_dir() -> PathBuf {
    PathBuf::from("reports/alerts")
}

fn dated_file(dir: &Path, ext: &str) -> PathBuf {
    let date = Local::now().format("%Y%m%d").to_string();
    dir.join(format!("{}.{}", date, ext))
}

fn write_jsonl(mut writer: impl Write, record: &AlertRecord) -> std::io::Result<()> {
    serde_json::to_writer(&mut writer, record).map_err(std::io::Error::other)?;
    writeln!(writer)
}

#[derive(Debug, Clone)]
pub struct AlertLog {
    dir: PathBuf,
    origin: AlertRecordOrigin,
    default_production: bool,
}

impl AlertLog {
    /// The production archive always uses the fixed reports/alerts namespace.
    pub fn production() -> Self {
        Self {
            dir: alerts_dir(),
            origin: AlertRecordOrigin::Production,
            default_production: true,
        }
    }

    /// Explicit isolated archive for tests. It never falls back to the production path.
    pub fn for_test(dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let dir = dir.into();
        let cwd = std::env::current_dir()?;
        let unresolved = if dir.is_absolute() {
            dir
        } else {
            cwd.join(dir)
        };
        let resolved = fs::canonicalize(unresolved)?;
        let production_path = cwd.join(alerts_dir());
        let production = fs::canonicalize(&production_path).unwrap_or(production_path);
        if resolved.starts_with(&production) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "test alert archive must be outside reports/alerts",
            ));
        }
        Ok(Self {
            dir: resolved,
            origin: AlertRecordOrigin::Test,
            default_production: false,
        })
    }

    #[cfg(test)]
    fn production_at(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            origin: AlertRecordOrigin::Production,
            default_production: false,
        }
    }

    fn ensure_io_allowed(&self) -> std::io::Result<()> {
        if self.default_production
            && (runtime_is_test_process() || current_env() == TradingEnv::Test)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "test runtime cannot access the production alert archive",
            ));
        }
        Ok(())
    }

    pub fn append_jsonl(&self, event: &AlertEvent) -> std::io::Result<()> {
        self.ensure_io_allowed()?;
        let record = AlertRecord::from_event(event, self.origin);
        if self.origin == AlertRecordOrigin::Production && !record.is_production_eligible() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "production alert archive rejected ineligible code: {}",
                    record.code
                ),
            ));
        }
        fs::create_dir_all(&self.dir)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dated_file(&self.dir, "jsonl"))?;
        write_jsonl(file, &record)
    }

    pub fn append_md(&self, event: &AlertEvent) -> std::io::Result<()> {
        self.ensure_io_allowed()?;
        if self.origin == AlertRecordOrigin::Production && is_test_code(&event.code) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "production alert archive rejected test code: {}",
                    event.code
                ),
            ));
        }
        fs::create_dir_all(&self.dir)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dated_file(&self.dir, "md"))?;
        write_markdown(file, event)
    }

    pub fn append_batch(&self, events: &[AlertEvent]) -> std::io::Result<()> {
        for event in events {
            self.append_jsonl(event)?;
            self.append_md(event)?;
        }
        Ok(())
    }

    pub fn read_today(&self) -> Vec<String> {
        if self.ensure_io_allowed().is_err() {
            return Vec::new();
        }
        match fs::read_to_string(dated_file(&self.dir, "md")) {
            Ok(s) => s
                .split("---\n")
                .filter(|p| !p.trim().is_empty())
                .map(|p| p.trim().to_string())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn today_stats(&self) -> (usize, usize, usize) {
        let alerts = self.read_today();
        let emergency = alerts.iter().filter(|a| a.contains("🔴")).count();
        let important = alerts.iter().filter(|a| a.contains("🟠")).count();
        let info = alerts.iter().filter(|a| a.contains("🟡")).count();
        (emergency, important, info)
    }

    pub fn read_today_records(&self) -> Vec<AlertRecord> {
        if self.ensure_io_allowed().is_err() {
            return Vec::new();
        }
        let Ok(content) = fs::read_to_string(dated_file(&self.dir, "jsonl")) else {
            return Vec::new();
        };
        content
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return None;
                }
                match serde_json::from_str::<AlertRecord>(trimmed) {
                    Ok(record)
                        if self.origin != AlertRecordOrigin::Production
                            || record.is_production_eligible() =>
                    {
                        Some(record)
                    }
                    Ok(record) => {
                        log::warn!(
                            "[alert_log] read_today_records 跳过非生产告警: code={} origin={:?}",
                            record.code,
                            record.origin
                        );
                        None
                    }
                    Err(error) => {
                        log::warn!("[alert_log] read_today_records 跳过无法解析的告警行: {error}");
                        None
                    }
                }
            })
            .collect()
    }
}

fn write_markdown(mut writer: impl Write, event: &AlertEvent) -> std::io::Result<()> {
    use crate::monitor::alert::format_alert;
    writeln!(writer, "---\n{}\n", format_alert(event))
}

/// 追加一条告警到 JSONL（机器可读）。打开、序列化或写入失败均显式返回。
pub fn append_jsonl(event: &AlertEvent) -> std::io::Result<()> {
    AlertLog::production().append_jsonl(event)
}

/// 追加一条告警到 Markdown（人可读）。打开或写入失败均显式返回。
pub fn append_md(event: &AlertEvent) -> std::io::Result<()> {
    AlertLog::production().append_md(event)
}

/// 批量追加（一次写入减少 IO）
pub fn append_batch(events: &[AlertEvent]) -> std::io::Result<()> {
    AlertLog::production().append_batch(events)
}

/// 读取今日告警
pub fn read_today() -> Vec<String> {
    AlertLog::production().read_today()
}

/// 今日告警统计
pub fn today_stats() -> (usize, usize, usize) {
    AlertLog::production().today_stats()
}

/// 读取今日告警 JSONL 结构化记录 (G5b 深链归因等机器消费方)。
/// 文件缺失或行解析失败 → 该行跳过 (出声: 返回错误行计数由调用方日志体现)。
pub fn read_today_records() -> Vec<AlertRecord> {
    AlertLog::production().read_today_records()
}

// ── JSON 记录 ──

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertRecordOrigin {
    #[default]
    LegacyUnknown,
    Production,
    Test,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct AlertRecord {
    #[serde(default)]
    pub origin: AlertRecordOrigin,
    pub triggered_at: String,
    pub code: String,
    pub name: String,
    pub level: String,
    pub category: String,
    pub message: String,
    pub price: Option<f64>,
    pub change_pct: Option<f64>,
    pub main_flow_yi: Option<f64>,
    pub news_title: Option<String>,
    pub news_importance: Option<u8>,
    pub attribution_decision: Option<String>,
    pub routed_external_id: Option<String>,
    pub t1_locked: bool,
}

impl AlertRecord {
    pub fn is_production_eligible(&self) -> bool {
        self.origin != AlertRecordOrigin::Test && !is_test_code(&self.code)
    }

    fn from_event(e: &AlertEvent, origin: AlertRecordOrigin) -> Self {
        AlertRecord {
            origin,
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
        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::for_test(temp.path()).unwrap();
        let event = e();
        archive.append_md(&event).unwrap();
        let alerts = archive.read_today();
        assert_eq!(alerts.len(), 1);
        assert!(alerts[0].contains("测试告警"));
    }

    #[test]
    fn test_append_jsonl() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let first_archive = AlertLog::for_test(first.path()).unwrap();
        let second_archive = AlertLog::for_test(second.path()).unwrap();

        first_archive.append_jsonl(&e()).unwrap();
        let path = dated_file(first.path(), "jsonl");
        assert!(path.exists());
        let records = first_archive.read_today_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].origin, AlertRecordOrigin::Test);
        assert!(second_archive.read_today_records().is_empty());
    }

    #[test]
    fn jsonl_contains_structured_attribution_evidence() {
        let mut event = e();
        event.detail.news_title = Some("TEST_CODE 快讯".into());
        event.detail.news_importance = Some(4);
        event.detail.ai_decision = Some("产业链催化 | 置信度B".into());
        let mut output = Vec::new();

        let record = AlertRecord::from_event(&event, AlertRecordOrigin::Test);
        write_jsonl(&mut output, &record).unwrap();

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

        let record = AlertRecord::from_event(&e(), AlertRecordOrigin::Test);
        assert!(write_jsonl(FailingWriter, &record).is_err());
    }

    #[test]
    fn test_today_stats() {
        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::for_test(temp.path()).unwrap();
        archive.append_md(&e()).unwrap();
        assert_eq!(archive.today_stats(), (0, 1, 0));
    }

    #[test]
    fn production_default_io_is_blocked_in_test_runtime() {
        let archive = AlertLog::production();
        assert_eq!(
            archive.append_jsonl(&e()).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert!(archive.read_today_records().is_empty());
    }

    #[test]
    fn test_archive_rejects_production_default_directory() {
        assert!(AlertLog::for_test(alerts_dir()).is_err());
        assert!(AlertLog::for_test("reports/../reports/alerts").is_err());
        let temp = tempfile::tempdir().unwrap();
        let alias = temp.path().join("production-alerts-alias");
        std::os::unix::fs::symlink(std::env::current_dir().unwrap().join(alerts_dir()), &alias)
            .unwrap();
        assert!(AlertLog::for_test(alias).is_err());
    }

    #[test]
    fn production_reader_admits_normal_legacy_and_rejects_test_origins_and_codes() {
        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::production_at(temp.path());
        let normal_legacy = serde_json::json!({
            "triggered_at":"2026-09-05T10:00:00+08:00","code":"600396","name":"正常旧记录",
            "level":"重要","category":"主力突袭","message":"normal","price":null,
            "change_pct":null,"main_flow_yi":null,"news_title":null,"news_importance":null,
            "attribution_decision":null,"routed_external_id":null,"t1_locked":false
        });
        let mut legacy_test_code = normal_legacy.clone();
        legacy_test_code["code"] = serde_json::json!("TEST_CODE_000001");
        let mut test_origin = normal_legacy.clone();
        test_origin["origin"] = serde_json::json!("test");
        test_origin["code"] = serde_json::json!("600001");
        let mut production = normal_legacy.clone();
        production["origin"] = serde_json::json!("production");
        production["code"] = serde_json::json!("000001");
        let content = [normal_legacy, legacy_test_code, test_origin, production]
            .into_iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::create_dir_all(temp.path()).unwrap();
        fs::write(dated_file(temp.path(), "jsonl"), format!("{content}\n")).unwrap();

        let records = archive.read_today_records();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].origin, AlertRecordOrigin::LegacyUnknown);
        assert_eq!(records[0].code, "600396");
        assert_eq!(records[1].origin, AlertRecordOrigin::Production);
        assert_eq!(records[1].code, "000001");
    }
}
