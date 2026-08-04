//! Registered business rules: BR-141.
//! v26: Dry-run 报告自动生成器
//!
//! 启动后台任务, 定时从 `data/dispatcher_log/*.jsonl` 汇总统计,
//! 写入 `data/dry_run_report.json`, 接在现有 run 过程中, 无新命令。
//!
//! 设计:
//! - tokio::spawn 后台循环, 默认每 5 分钟一次
//! - 读今日 dispatch_log (JSONL) + 1d 历史
//! - 汇总: 按 kind 推送量/成功率, 数据源健康, 主题 top-5
//! - 输出: 单文件 JSON (machine readable, 后续可视化)

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

/// 报告数据 (整体序列化)
#[derive(Debug, Serialize)]
pub struct DryRunReport {
    pub generated_at: String,   // ISO8601 时间戳
    pub window_hours: u64,      // 报告时间窗口 (默认 24h)
    pub total_attempts: u64,    // 总推送尝试
    pub success_rate: f64,      // 成功率 (0.0-1.0)
    pub by_kind: Vec<KindStat>, // 按模板统计
    pub source_health: Vec<SourceStat>,
    pub top_topics: Vec<TopicStat>, // top 5 主题命中
}

/// 按模板统计
#[derive(Debug, Serialize)]
pub struct KindStat {
    pub kind: String,
    pub total: u64,
    pub success: u64,
    pub failed: u64,
}

/// 数据源健康
#[derive(Debug, Serialize)]
pub struct SourceStat {
    pub source: String,
    pub attempts: u64,
    pub empty: u64,
    pub errors: u64,
}

/// 主题命中
#[derive(Debug, Serialize)]
pub struct TopicStat {
    pub topic: String,
    pub count: u64,
}

/// 启动后台 dry-run 报告生成器
/// interval: 报告刷新间隔 (默认 5 min)
pub fn spawn_dryrun_reporter(interval_secs: u64) -> tokio::task::JoinHandle<()> {
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
        // 跳过第一个 tick (立即触发)
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(e) = generate_report().await {
                log::warn!("[v26 dryrun] 报告生成失败: {}", e);
            }
        }
    });
    log::info!(
        "[v26 dryrun] 后台报告生成器已启动 (interval: {}s)",
        interval_secs
    );
    handle
}

/// 生成一次报告 (立即调用, 也被后台 task 调用)
pub async fn generate_report() -> anyhow::Result<()> {
    let report = collect_report().await?;
    let json = serde_json::to_string_pretty(&report)?;
    let path = "data/dry_run_report.json";
    std::fs::create_dir_all("data").ok();
    let len = json.len();
    std::fs::write(path, json)?;
    log::debug!("[v26 dryrun] 报告已写入 {} ({} 字节)", path, len);
    Ok(())
}

/// 收集所有统计 (读 dispatcher_log/*.jsonl)
async fn collect_report() -> anyhow::Result<DryRunReport> {
    collect_report_from(Path::new("data/dispatcher_log")).await
}

/// 从已解析的日志目录收集统计。
///
/// 生产入口固定传入 `data/dispatcher_log`；显式目录参数让测试可以在隔离目录
/// 执行同一 JSONL 校验与聚合逻辑，而不读取真实投递审计。
async fn collect_report_from(log_dir: &Path) -> anyhow::Result<DryRunReport> {
    let mut total_attempts = 0u64;
    let mut total_success = 0u64;
    let mut by_kind_map: HashMap<String, KindStat> = HashMap::new();
    let mut by_source_map: HashMap<String, SourceStat> = HashMap::new();
    let mut by_topic_map: HashMap<String, u64> = HashMap::new();

    // 读所有 dispatcher_log 文件
    if log_dir.is_dir() {
        let mut entries = tokio::fs::read_dir(log_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let content = tokio::fs::read_to_string(&path).await?;
            for (line_index, line) in content.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let record: serde_json::Value = serde_json::from_str(line).map_err(|error| {
                    anyhow::anyhow!(
                        "dispatcher 日志 {} 第 {} 行不是合法 JSON: {}",
                        path.display(),
                        line_index + 1,
                        error
                    )
                })?;
                let object = record.as_object().ok_or_else(|| {
                    anyhow::anyhow!(
                        "dispatcher 日志 {} 第 {} 行不是对象",
                        path.display(),
                        line_index + 1
                    )
                })?;
                let kind = object
                    .get("kind")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "dispatcher 日志 {} 第 {} 行缺少非空 kind",
                            path.display(),
                            line_index + 1
                        )
                    })?
                    .to_string();
                let success = object
                    .get("success")
                    .and_then(|value| value.as_bool())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "dispatcher 日志 {} 第 {} 行缺少布尔 success",
                            path.display(),
                            line_index + 1
                        )
                    })?;
                let error = match object.get("error") {
                    None => "",
                    Some(value) => value.as_str().ok_or_else(|| {
                        anyhow::anyhow!(
                            "dispatcher 日志 {} 第 {} 行 error 不是字符串",
                            path.display(),
                            line_index + 1
                        )
                    })?,
                };

                total_attempts += 1;
                if success {
                    total_success += 1;
                }

                let stat = by_kind_map.entry(kind.clone()).or_insert(KindStat {
                    kind: kind.clone(),
                    total: 0,
                    success: 0,
                    failed: 0,
                });
                stat.total += 1;
                if success {
                    stat.success += 1;
                } else {
                    stat.failed += 1;
                }

                // 数据源 health: 从 kind 前缀推断 (e.g. "P-01-dry" → dryrun, "I-02" → news)
                if let Some(source) = source_from_kind(&kind) {
                    let s = by_source_map.entry(source.clone()).or_insert(SourceStat {
                        source: source.clone(),
                        attempts: 0,
                        empty: 0,
                        errors: 0,
                    });
                    s.attempts += 1;
                    if error.contains("空") || error.contains("无数据") {
                        s.empty += 1;
                    } else if !success && !error.is_empty() {
                        s.errors += 1;
                    }
                }
            }
        }
    }

    // top 5 topics (简化: 从 kind 中提取主题关键词, 实际项目应从 chain_mapper 输出拿)
    // v26 简化版: 暂用 kind 名称作为主题代理
    for (kind, stat) in &by_kind_map {
        if kind.contains("dry") {
            // dryrun 测试数据, 跳过
            continue;
        }
        *by_topic_map.entry(kind.clone()).or_insert(0) += stat.total;
    }

    let mut top_topics: Vec<TopicStat> = by_topic_map
        .into_iter()
        .map(|(t, c)| TopicStat { topic: t, count: c })
        .collect();
    top_topics.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.topic.cmp(&right.topic))
    });
    top_topics.truncate(5);

    let mut by_kind: Vec<KindStat> = by_kind_map.into_values().collect();
    by_kind.sort_by(|left, right| {
        right
            .total
            .cmp(&left.total)
            .then_with(|| left.kind.cmp(&right.kind))
    });

    let mut source_health: Vec<SourceStat> = by_source_map.into_values().collect();
    source_health.sort_by(|left, right| {
        right
            .attempts
            .cmp(&left.attempts)
            .then_with(|| left.source.cmp(&right.source))
    });

    let success_rate = if total_attempts > 0 {
        total_success as f64 / total_attempts as f64
    } else {
        0.0
    };

    Ok(DryRunReport {
        generated_at: chrono::Local::now().to_rfc3339(),
        window_hours: 24,
        total_attempts,
        success_rate,
        by_kind,
        source_health,
        top_topics,
    })
}

/// 从 kind 推断数据源
fn source_from_kind(kind: &str) -> Option<String> {
    if kind.contains("dry") {
        Some("dryrun_test".to_string())
    } else if kind.starts_with("P-01") || kind.contains("盘前") {
        Some("东方财富".to_string())
    } else if kind.starts_with("I-01") {
        Some("sector_monitor".to_string())
    } else if kind.starts_with("I-") || kind.starts_with("R-") {
        Some("search_service".to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stock-analysis-dryrun-{label}-{}-{id}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn collect_report_aggregates_strict_jsonl_in_an_isolated_directory() {
        let dir = TestDir::new("aggregate");
        std::fs::write(
            dir.path().join("2026-07-18.jsonl"),
            concat!(
                "{\"kind\":\"I-01-sector\",\"success\":true}\n",
                "{\"kind\":\"I-01-sector\",\"success\":false,\"error\":\"无数据\"}\n",
                "{\"kind\":\"P-01-dry\",\"success\":true}\n",
                "{\"kind\":\"R-02-review\",\"success\":false,\"error\":\"timeout\"}\n",
                "\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.path().join("ignored.txt"), "not jsonl").unwrap();

        let report = collect_report_from(dir.path()).await.unwrap();
        assert_eq!(report.total_attempts, 4);
        assert_eq!(report.success_rate, 0.5);
        assert_eq!(report.by_kind[0].kind, "I-01-sector");
        assert_eq!(report.by_kind[0].total, 2);
        assert_eq!(report.by_kind[0].success, 1);
        assert_eq!(report.by_kind[0].failed, 1);
        assert!(report
            .top_topics
            .iter()
            .all(|item| !item.topic.contains("dry")));

        let dry = report
            .source_health
            .iter()
            .find(|item| item.source == "dryrun_test")
            .unwrap();
        assert_eq!(dry.attempts, 1);
        assert!(!report
            .source_health
            .iter()
            .any(|item| item.source == "东方财富"));
        let sector = report
            .source_health
            .iter()
            .find(|item| item.source == "sector_monitor")
            .unwrap();
        assert_eq!(sector.empty, 1);
        let search = report
            .source_health
            .iter()
            .find(|item| item.source == "search_service")
            .unwrap();
        assert_eq!(search.errors, 1);
    }

    #[tokio::test]
    async fn collect_report_rejects_corrupt_or_incomplete_audit_rows() {
        let cases = [
            ("{bad json}", "不是合法 JSON"),
            ("[]", "不是对象"),
            (r#"{"success":true}"#, "非空 kind"),
            (r#"{"kind":"I-01"}"#, "布尔 success"),
            (
                r#"{"kind":"I-01","success":false,"error":7}"#,
                "error 不是字符串",
            ),
        ];

        for (index, (line, expected)) in cases.into_iter().enumerate() {
            let dir = TestDir::new(&format!("invalid-{index}"));
            std::fs::write(dir.path().join(format!("case-{index}.jsonl")), line).unwrap();
            let error = collect_report_from(dir.path()).await.unwrap_err();
            assert!(error.to_string().contains(expected), "{error:#}");
        }
    }

    #[test]
    fn source_classification_prioritizes_dryrun_and_covers_all_prefixes() {
        assert_eq!(source_from_kind("P-01-dry").as_deref(), Some("dryrun_test"));
        assert_eq!(source_from_kind("P-01").as_deref(), Some("东方财富"));
        assert_eq!(source_from_kind("盘前").as_deref(), Some("东方财富"));
        assert_eq!(source_from_kind("I-01").as_deref(), Some("sector_monitor"));
        assert_eq!(source_from_kind("I-02").as_deref(), Some("search_service"));
        assert_eq!(source_from_kind("R-01").as_deref(), Some("search_service"));
        assert_eq!(source_from_kind("X-01"), None);
    }
}
