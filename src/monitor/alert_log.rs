//! Registered business rules: BR-045.
//! 告警本地归档：每条告警落盘为 JSONL + Markdown 双格式。
//! 路径：reports/alerts/{date}.jsonl  +  reports/alerts/{date}.md

use crate::monitor::detector::AlertEvent;
use crate::risk::env_guard::{current_env, is_test_code, runtime_is_test_process, TradingEnv};
use chrono::{Local, NaiveDate};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

mod input_head;
pub(crate) use input_head::DateFence as G5bDateFence;
pub use input_head::{
    inspect_date_input_head, AlertInputHeadUnknown, AlertInputHeadV1, VerifiedAlertInputPrefix,
};
pub(crate) use input_head::{
    LockedAlertInputPrefix, VerifiedAlertInputCutoff, VerifiedAlertInputLine,
};

fn alerts_dir() -> PathBuf {
    crate::production_root::production_root().join("reports/alerts")
}

fn dated_file(dir: &Path, ext: &str) -> PathBuf {
    let date = Local::now().format("%Y%m%d").to_string();
    dir.join(format!("{}.{}", date, ext))
}

fn dated_file_for(dir: &Path, ext: &str, date: NaiveDate) -> PathBuf {
    dir.join(format!("{}.{}", date.format("%Y%m%d"), ext))
}

/// A strict parse of one dated alert file, without a writer fence or a durable input head.
/// Even these rows cannot prove a stable day cutoff or authorize an empty-day seal.
#[derive(Debug)]
pub struct UnfencedAlertRecords {
    date: NaiveDate,
    records: Vec<AlertRecord>,
}

impl UnfencedAlertRecords {
    pub fn date(&self) -> NaiveDate {
        self.date
    }

    pub fn records(&self) -> &[AlertRecord] {
        &self.records
    }
}

/// A dated alert file whose complete contents cannot be established by the strict reader.
#[derive(Debug)]
pub enum AlertInputUnknown {
    AccessDenied,
    Missing,
    NonRegular,
    ChangedDuringRead,
    EmptyWithoutHead,
    TruncatedFinalLine,
    MalformedLine {
        line: usize,
        source: serde_json::Error,
    },
    IneligibleRecord {
        line: usize,
    },
    Io(std::io::Error),
}

fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if left.dev() != right.dev()
            || left.ino() != right.ino()
            || left.ctime() != right.ctime()
            || left.ctime_nsec() != right.ctime_nsec()
        {
            return false;
        }
    }
    left.is_file()
        && right.is_file()
        && left.len() == right.len()
        && matches!((left.modified(), right.modified()), (Ok(a), Ok(b)) if a == b)
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
        let production_path = alerts_dir();
        let production = fs::canonicalize(&production_path).unwrap_or(production_path);
        // Keep the old CWD-relative isolation boundary too: a build-root
        // override must not make a formerly forbidden archive a test fixture.
        let legacy_path = cwd.join("reports/alerts");
        let legacy_production = fs::canonicalize(&legacy_path).unwrap_or(legacy_path);
        if resolved.starts_with(&production) || resolved.starts_with(&legacy_production) {
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
        self.append_date_jsonl(Local::now().date_naive(), event)
    }

    pub fn append_md(&self, event: &AlertEvent) -> std::io::Result<()> {
        self.append_date_md(Local::now().date_naive(), event)
    }

    fn append_date_md(&self, date: NaiveDate, event: &AlertEvent) -> std::io::Result<()> {
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
            .open(dated_file_for(&self.dir, "md", date))?;
        write_markdown(file, event)
    }

    pub fn append_batch(&self, events: &[AlertEvent]) -> std::io::Result<()> {
        self.append_date_batch(Local::now().date_naive(), events)
    }

    fn append_date_batch(&self, date: NaiveDate, events: &[AlertEvent]) -> std::io::Result<()> {
        for event in events {
            self.append_date_jsonl(date, event)?;
            self.append_date_md(date, event)?;
        }
        Ok(())
    }

    pub fn read_today(&self) -> Vec<String> {
        if let Err(error) = self.ensure_io_allowed() {
            log::warn!("[alert_log] read_today 拒绝读取默认生产归档: {error}");
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
        if let Err(error) = self.ensure_io_allowed() {
            log::warn!("[alert_log] read_today_records 拒绝读取默认生产归档: {error}");
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

    /// Parse every line of an explicit business-date JSONL file. This is an unfenced
    /// observation: concurrent appends or later replacement remain possible. In particular,
    /// missing or empty input is Unknown until a durable input head exists.
    pub fn inspect_date_records_strict(
        &self,
        date: NaiveDate,
    ) -> Result<UnfencedAlertRecords, AlertInputUnknown> {
        self.ensure_io_allowed().map_err(|error| {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                AlertInputUnknown::AccessDenied
            } else {
                AlertInputUnknown::Io(error)
            }
        })?;

        let path = dated_file_for(&self.dir, "jsonl", date);
        let before = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(AlertInputUnknown::Missing);
            }
            Err(error) => return Err(AlertInputUnknown::Io(error)),
        };
        if !before.is_file() {
            return Err(AlertInputUnknown::NonRegular);
        }

        let mut file = fs::File::open(&path).map_err(AlertInputUnknown::Io)?;
        let opened = file.metadata().map_err(AlertInputUnknown::Io)?;
        if !same_file_state(&before, &opened) {
            return Err(AlertInputUnknown::ChangedDuringRead);
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(AlertInputUnknown::Io)?;
        let after_file = file.metadata().map_err(AlertInputUnknown::Io)?;
        let after_path =
            fs::symlink_metadata(&path).map_err(|_| AlertInputUnknown::ChangedDuringRead)?;
        if !same_file_state(&before, &after_file)
            || !same_file_state(&before, &after_path)
            || bytes.len() as u64 != before.len()
        {
            return Err(AlertInputUnknown::ChangedDuringRead);
        }

        if bytes.is_empty() {
            return Err(AlertInputUnknown::EmptyWithoutHead);
        }
        if bytes.last() != Some(&b'\n') {
            return Err(AlertInputUnknown::TruncatedFinalLine);
        }

        let mut records = Vec::new();
        for (index, line) in bytes[..bytes.len() - 1]
            .split(|byte| *byte == b'\n')
            .enumerate()
        {
            let line_number = index + 1;
            let record: AlertRecord = serde_json::from_slice(line).map_err(|source| {
                AlertInputUnknown::MalformedLine {
                    line: line_number,
                    source,
                }
            })?;
            if self.origin == AlertRecordOrigin::Production && !record.is_production_eligible() {
                return Err(AlertInputUnknown::IneligibleRecord { line: line_number });
            }
            records.push(record);
        }
        Ok(UnfencedAlertRecords { date, records })
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

/// Strict explicit-date read for future fenced G5b input provenance work.
pub fn inspect_date_records_strict(
    date: NaiveDate,
) -> Result<UnfencedAlertRecords, AlertInputUnknown> {
    AlertLog::production().inspect_date_records_strict(date)
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

    fn past_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 5).unwrap()
    }

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
    fn ordinary_jsonl_writer_publishes_and_advances_exact_head() {
        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::for_test(temp.path()).unwrap();
        archive.append_jsonl(&e()).unwrap();
        archive.append_jsonl(&e()).unwrap();
        let date = Local::now().date_naive();
        let snapshot = archive.inspect_date_input_head(date).unwrap();
        let bytes = fs::read(dated_file_for(temp.path(), "jsonl", date)).unwrap();
        assert_eq!(snapshot.records().len(), 2);
        assert_eq!(snapshot.head().generation(), 2);
        assert_eq!(snapshot.head().committed_offset(), bytes.len() as u64);
        use sha2::{Digest, Sha256};
        assert_eq!(
            snapshot.head().prefix_sha256(),
            hex::encode(Sha256::digest(bytes))
        );
    }

    #[test]
    fn batch_uses_one_explicit_date_for_jsonl_and_markdown() {
        use chrono::TimeZone;
        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::for_test(temp.path()).unwrap();
        let date = past_date();
        let mut first = e();
        first.code = "TEST_CODE_FIRST".into();
        first.triggered_at = Local
            .with_ymd_and_hms(2026, 10, 1, 23, 59, 59)
            .single()
            .unwrap();
        let mut second = e();
        second.code = "TEST_CODE_SECOND".into();
        second.triggered_at = Local
            .with_ymd_and_hms(2026, 10, 2, 0, 0, 1)
            .single()
            .unwrap();
        archive.append_date_batch(date, &[first, second]).unwrap();
        let snapshot = archive.inspect_date_input_head(date).unwrap();
        assert_eq!(snapshot.head().generation(), 2);
        assert_eq!(snapshot.records()[0].code, "TEST_CODE_FIRST");
        assert_eq!(snapshot.records()[1].code, "TEST_CODE_SECOND");
        let markdown = fs::read_to_string(dated_file_for(temp.path(), "md", date)).unwrap();
        assert!(markdown.contains("TEST_CODE_FIRST"));
        assert!(markdown.contains("TEST_CODE_SECOND"));
        for other_date in [
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
        ] {
            assert!(!dated_file_for(temp.path(), "jsonl", other_date).exists());
            assert!(!dated_file_for(temp.path(), "md", other_date).exists());
        }
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
    fn public_archive_append_operations_return_path_io_failures() {
        let json_temp = tempfile::tempdir().unwrap();
        fs::create_dir(dated_file(json_temp.path(), "jsonl")).unwrap();
        let json_archive = AlertLog::for_test(json_temp.path()).unwrap();
        assert!(json_archive.append_jsonl(&e()).is_err());

        let md_temp = tempfile::tempdir().unwrap();
        fs::create_dir(dated_file(md_temp.path(), "md")).unwrap();
        let md_archive = AlertLog::for_test(md_temp.path()).unwrap();
        assert!(md_archive.append_md(&e()).is_err());

        let batch_temp = tempfile::tempdir().unwrap();
        fs::create_dir(dated_file(batch_temp.path(), "jsonl")).unwrap();
        let batch_archive = AlertLog::for_test(batch_temp.path()).unwrap();
        assert!(batch_archive.append_batch(&[e()]).is_err());
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
        assert!(archive.read_today().is_empty());
        assert!(archive.read_today_records().is_empty());
    }

    #[test]
    fn production_namespace_is_fixed_across_process_cwd() {
        const CHILD_ENV: &str = "STOCK_ANALYSIS_TEST_ALERT_LOG_FIXED_ROOT_CHILD";
        if std::env::var_os(CHILD_ENV).is_some() {
            let archive = AlertLog::production();
            assert_eq!(
                archive.dir,
                crate::production_root::production_root().join("reports/alerts")
            );
            assert!(archive.dir.is_absolute());
            assert_ne!(
                archive.dir,
                std::env::current_dir().unwrap().join("reports/alerts")
            );
            assert_eq!(
                archive.append_jsonl(&e()).unwrap_err().kind(),
                std::io::ErrorKind::PermissionDenied
            );
            assert_eq!(
                archive.append_md(&e()).unwrap_err().kind(),
                std::io::ErrorKind::PermissionDenied
            );
            assert!(matches!(
                archive.acquire_date_writer_fence(Local::now().date_naive()),
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied
            ));
            for forbidden in ["reports/alerts", "reports/../reports/alerts"] {
                assert_eq!(
                    AlertLog::for_test(forbidden).unwrap_err().kind(),
                    std::io::ErrorKind::InvalidInput
                );
            }
            #[cfg(unix)]
            assert_eq!(
                AlertLog::for_test("legacy-alerts-alias")
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::InvalidInput
            );
            println!("fixed-root-alert-child-verified");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let legacy_dir = root.path().join("reports/alerts");
        fs::create_dir_all(&legacy_dir).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&legacy_dir, root.path().join("legacy-alerts-alias")).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "monitor::alert_log::tests::production_namespace_is_fixed_across_process_cwd",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .env("STOCK_ANALYSIS_BUILD_PRODUCTION_ROOT", root.path())
            .current_dir(root.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixed-root child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("fixed-root-alert-child-verified"));
        assert!(fs::read_dir(&legacy_dir).unwrap().next().is_none());
        assert!(!root.path().join("data").exists());
    }

    #[test]
    fn test_archive_rejects_production_default_directory() {
        assert!(AlertLog::for_test(alerts_dir()).is_err());
        assert!(AlertLog::for_test("reports/../reports/alerts").is_err());
        #[cfg(unix)]
        {
            let temp = tempfile::tempdir().unwrap();
            let alias = temp.path().join("production-alerts-alias");
            std::os::unix::fs::symlink(std::env::current_dir().unwrap().join(alerts_dir()), &alias)
                .unwrap();
            assert!(AlertLog::for_test(alias).is_err());
        }
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
        let mut unknown_origin = normal_legacy.clone();
        unknown_origin["origin"] = serde_json::json!("future_origin");
        unknown_origin["code"] = serde_json::json!("600002");
        let content = [
            normal_legacy,
            legacy_test_code,
            test_origin,
            production,
            unknown_origin,
        ]
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

    #[test]
    fn strict_date_reader_never_proves_an_empty_day_without_a_head() {
        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::for_test(temp.path()).unwrap();
        let date = past_date();
        assert!(matches!(
            archive.inspect_date_records_strict(date),
            Err(AlertInputUnknown::Missing)
        ));

        let path = dated_file_for(temp.path(), "jsonl", date);
        fs::write(&path, []).unwrap();
        assert!(matches!(
            archive.inspect_date_records_strict(date),
            Err(AlertInputUnknown::EmptyWithoutHead)
        ));

        let mut first = AlertRecord::from_event(&e(), AlertRecordOrigin::Test);
        first.code = "TEST_CODE_FIRST".into();
        let mut second = first.clone();
        second.code = "TEST_CODE_SECOND".into();
        let mut bytes = Vec::new();
        write_jsonl(&mut bytes, &first).unwrap();
        write_jsonl(&mut bytes, &second).unwrap();
        fs::write(path, bytes).unwrap();

        let observed = archive.inspect_date_records_strict(date).unwrap();
        assert_eq!(observed.date(), date);
        assert_eq!(observed.records().len(), 2);
        assert_eq!(observed.records()[0].code, "TEST_CODE_FIRST");
        assert_eq!(observed.records()[1].code, "TEST_CODE_SECOND");
    }

    #[test]
    fn strict_date_reader_rejects_partial_and_malformed_rows() {
        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::for_test(temp.path()).unwrap();
        let path = dated_file_for(temp.path(), "jsonl", past_date());
        let mut valid = Vec::new();
        write_jsonl(
            &mut valid,
            &AlertRecord::from_event(&e(), AlertRecordOrigin::Test),
        )
        .unwrap();

        let mut partial = valid.clone();
        partial.extend_from_slice(b"{\"code\":");
        fs::write(&path, partial).unwrap();
        assert!(matches!(
            archive.inspect_date_records_strict(past_date()),
            Err(AlertInputUnknown::TruncatedFinalLine)
        ));

        let mut malformed = valid.clone();
        malformed.extend_from_slice(b"{\"code\":}\n");
        fs::write(&path, malformed).unwrap();
        assert!(matches!(
            archive.inspect_date_records_strict(past_date()),
            Err(AlertInputUnknown::MalformedLine { line: 2, .. })
        ));

        valid.push(b'\n');
        fs::write(path, valid).unwrap();
        assert!(matches!(
            archive.inspect_date_records_strict(past_date()),
            Err(AlertInputUnknown::MalformedLine { line: 2, .. })
        ));
    }

    #[test]
    fn strict_date_reader_rejects_nonregular_and_ineligible_production_input() {
        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::production_at(temp.path());
        let path = dated_file_for(temp.path(), "jsonl", past_date());
        fs::create_dir(&path).unwrap();
        assert!(matches!(
            archive.inspect_date_records_strict(past_date()),
            Err(AlertInputUnknown::NonRegular)
        ));
        fs::remove_dir(&path).unwrap();

        let mut record = AlertRecord::from_event(&e(), AlertRecordOrigin::Production);
        let mut bytes = Vec::new();
        write_jsonl(&mut bytes, &record).unwrap();
        fs::write(&path, bytes).unwrap();
        assert!(matches!(
            archive.inspect_date_records_strict(past_date()),
            Err(AlertInputUnknown::IneligibleRecord { line: 1 })
        ));

        record.code = "600396".into();
        let mut bytes = Vec::new();
        write_jsonl(&mut bytes, &record).unwrap();
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            archive
                .inspect_date_records_strict(past_date())
                .unwrap()
                .records()[0]
                .code,
            "600396"
        );

        #[cfg(unix)]
        {
            fs::remove_file(&path).unwrap();
            let target = temp.path().join("target.jsonl");
            fs::write(&target, b"{}\n").unwrap();
            std::os::unix::fs::symlink(target, path).unwrap();
            assert!(matches!(
                archive.inspect_date_records_strict(past_date()),
                Err(AlertInputUnknown::NonRegular)
            ));
        }
    }

    #[test]
    fn strict_date_reader_respects_production_test_runtime_guard() {
        assert!(matches!(
            AlertLog::production().inspect_date_records_strict(past_date()),
            Err(AlertInputUnknown::AccessDenied)
        ));
    }
}
