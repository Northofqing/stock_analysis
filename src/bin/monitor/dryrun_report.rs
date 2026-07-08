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
    pub generated_at: String,        // ISO8601 时间戳
    pub window_hours: u64,           // 报告时间窗口 (默认 24h)
    pub total_attempts: u64,         // 总推送尝试
    pub success_rate: f64,           // 成功率 (0.0-1.0)
    pub by_kind: Vec<KindStat>,       // 按模板统计
    pub source_health: Vec<SourceStat>,
    pub top_topics: Vec<TopicStat>,   // top 5 主题命中
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
pub fn spawn_dryrun_reporter(interval_secs: u64) {
    tokio::spawn(async move {
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
    log::info!("[v26 dryrun] 后台报告生成器已启动 (interval: {}s)", interval_secs);
}

/// v14.1 task #162: 启动后台 backfill 调度器
///   每个交易日 15:30 跑一次 backfill_recommendations_outcome(yesterday)
///   - D 日推送 → D+1 收盘后 (15:30) 自动算 outcome
///   - 非交易日不跑 (calendar::is_trading_day)
///   - interval: 1 min (粗粒度, 触发后当天不再跑)
pub fn spawn_outcome_backfill_scheduler() {
    tokio::spawn(async move {
        // 等 1 min 让 DB 起来
        tokio::time::sleep(Duration::from_secs(60)).await;
        let mut last_run_date: Option<String> = None;
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.tick().await; // 跳过第一个
        loop {
            ticker.tick().await;
            let now = chrono::Local::now();
            let today = now.date_naive();
            // 触发条件: 15:30 后 + 交易日 + 今日没跑过
            if now.time() < chrono::NaiveTime::from_hms_opt(15, 30, 0).unwrap() {
                continue;
            }
            if !crate::calendar::is_trading_day(today) {
                continue;
            }
            let today_str = today.format("%Y-%m-%d").to_string();
            if last_run_date.as_deref() == Some(&today_str) {
                continue;
            }
            // 回填昨天 (D 日推送 → 今天 D+1 收盘)
            let yesterday = crate::calendar::prev_trading_day(today);
            let date_str = yesterday.format("%Y-%m-%d").to_string();
            log::info!("[v14.1 #162] 自动 backfill outcome | 日期 = {}", date_str);
            let updated = stock_analysis::opportunity::news_outcome::backfill_recommendations_outcome(&date_str);
            log::info!("[v14.1 #162] 自动 backfill 完成 | {} | 更新 {} 条", date_str, updated);
            last_run_date = Some(today_str);
        }
    });
    log::info!("[v14.1 #162] 后台 outcome backfill 调度器已启动 (15:30 每日)");
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
    let mut total_attempts = 0u64;
    let mut total_success = 0u64;
    let mut by_kind_map: HashMap<String, KindStat> = HashMap::new();
    let mut by_source_map: HashMap<String, SourceStat> = HashMap::new();
    let mut by_topic_map: HashMap<String, u64> = HashMap::new();

    // 读所有 dispatcher_log 文件
    let log_dir = Path::new("data/dispatcher_log");
    if log_dir.is_dir() {
        let mut entries = tokio::fs::read_dir(log_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let content = tokio::fs::read_to_string(&path).await?;
            for line in content.lines() {
                if let Ok(record) = serde_json::from_str::<serde_json::Value>(line) {
                    let kind = record.get("kind").and_then(|k| k.as_str()).unwrap_or("").to_string();
                    let success = record.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
                    let error = record.get("error").and_then(|e| e.as_str()).unwrap_or("");

                    total_attempts += 1;
                    if success {
                        total_success += 1;
                    }

                    let stat = by_kind_map.entry(kind.clone()).or_insert(KindStat {
                        kind: kind.clone(),
                        total: 0, success: 0, failed: 0,
                    });
                    stat.total += 1;
                    if success { stat.success += 1; } else { stat.failed += 1; }

                    // 数据源 health: 从 kind 前缀推断 (e.g. "P-01-dry" → dryrun, "I-02" → news)
                    if let Some(source) = source_from_kind(&kind) {
                        let s = by_source_map.entry(source.clone()).or_insert(SourceStat {
                            source: source.clone(),
                            attempts: 0, empty: 0, errors: 0,
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
    top_topics.sort_by(|a, b| b.count.cmp(&a.count));
    top_topics.truncate(5);

    let mut by_kind: Vec<KindStat> = by_kind_map.into_values().collect();
    by_kind.sort_by(|a, b| b.total.cmp(&a.total));

    let mut source_health: Vec<SourceStat> = by_source_map.into_values().collect();
    source_health.sort_by(|a, b| b.attempts.cmp(&a.attempts));

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
    if kind.starts_with("P-01") || kind.contains("盘前") {
        Some("东方财富".to_string())
    } else if kind.starts_with("I-01") {
        Some("sector_monitor".to_string())
    } else if kind.contains("dry") {
        Some("dryrun_test".to_string())
    } else if kind.starts_with("I-") || kind.starts_with("R-") {
        Some("search_service".to_string())
    } else {
        None
    }
}
