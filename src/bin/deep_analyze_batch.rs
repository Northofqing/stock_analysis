//! 盘后多 Agent 研判批次（方案 A，2026-11-12）。
//!
//! 独立于 19:00 复盘 R-11：在日线落库后，对“当前持仓 + 少量 open 提案”
//! 跑现有 deep_analyzer 多分析师流水线。只写报告与 sidecar，不交易、不推送。
//!
//! 用法（cwd=运行根）：
//!   target/release/deep_analyze_batch --db data/stock_analysis.db \
//!       --proposal-limit 3 --max 9 --concurrency 2
//!   target/release/deep_analyze_batch --dry-run
//!
//! 选择口径：
//! - 持仓：最新 user_position_snapshot 的全部 code，按数量×成本降序；
//! - 提案：active entry_proposed 且最新 decision 为 open，按最新成交额降序；
//! - 去重、截断到 --max。
//!
//! 数据审计写独立 probe DB，不把分析产生的 acquisition audit 写进主库。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use serde::Serialize;

#[derive(Debug, Clone)]
struct Candidate {
    code: String,
    name: String,
    source: &'static str,
}

#[derive(Debug, Serialize)]
struct Sidecar {
    code: String,
    name: String,
    source: String,
    status: String,
    elapsed_secs: f64,
    report_path: Option<String>,
    composite: Option<i32>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct BatchSummary {
    business_date: String,
    started_at: String,
    finished_at: String,
    selected: usize,
    concurrency: usize,
    timeout_secs: u64,
    budget_exceeded: bool,
    results: Vec<Sidecar>,
}

#[derive(Debug, Serialize)]
struct Selected {
    business_date: String,
    candidates: Vec<SelectedRow>,
}

#[derive(Debug, Serialize)]
struct SelectedRow {
    code: String,
    name: String,
    source: String,
}

fn select_candidates(
    db_path: &Path,
    business_date: &str,
    proposal_limit: usize,
    max: usize,
) -> Result<Vec<Candidate>> {
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .with_context(|| format!("打开主库失败: {}", db_path.display()))?;

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut holding_stmt = conn.prepare(
        "SELECT code, name FROM user_position_snapshot_item \
         WHERE snapshot_id = (SELECT snapshot_id FROM user_position_snapshot \
                              ORDER BY effective_at DESC LIMIT 1) \
         ORDER BY quantity * cost_price DESC, code",
    )?;
    let holdings = holding_stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in holdings {
        let (code, name) = row?;
        if seen.insert(code.clone()) {
            candidates.push(Candidate {
                code,
                name,
                source: "holding",
            });
        }
    }

    let mut proposal_stmt = conn.prepare(
        "SELECT t.code, \
                COALESCE((SELECT name FROM user_position_snapshot_item i \
                          WHERE i.code=t.code ORDER BY rowid DESC LIMIT 1), t.code) AS name, \
                (SELECT d.amount FROM stock_daily d WHERE d.code=t.code \
                 ORDER BY d.date DESC LIMIT 1) AS amount \
         FROM ticket_lifecycle t \
         JOIN ticket_decision td ON td.ticket_id=t.id \
         WHERE t.close_date IS NULL AND t.stage='entry_proposed' \
           AND td.as_of=(SELECT MAX(as_of) FROM ticket_decision \
                         WHERE ticket_id=t.id AND as_of<=?) \
           AND td.proposal_status='open' \
           AND NOT EXISTS ( \
               SELECT 1 FROM ticket_event e WHERE e.ticket_id=t.id \
                 AND e.event_kind IN ('entry_triggered','entry_invalidated','entry_expired') \
           ) \
         GROUP BY t.code ORDER BY amount DESC NULLS LAST, t.code LIMIT ?",
    )?;
    let proposals = proposal_stmt.query_map(
        rusqlite::params![business_date, proposal_limit as i64],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;
    for row in proposals {
        let (code, name) = row?;
        if seen.insert(code.clone()) {
            candidates.push(Candidate {
                code,
                name,
                source: "proposal",
            });
        }
    }

    candidates.truncate(max);
    Ok(candidates)
}

fn parse_composite(report: &str) -> Option<i32> {
    for line in report.lines() {
        for key in ["综合评分", "加权综合", "composite_score", "composite", "Composite"] {
            if let Some(pos) = line.find(key) {
                let tail = &line[pos + key.len()..];
                let tail = tail.trim_start_matches(|c: char| {
                    c == ':' || c == '：' || c == ' ' || c == '=' || c == '*'
                });
                let number: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(value) = number.parse::<i32>() {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn write_sidecar(report_path: &Path, sidecar: &Sidecar) -> Result<()> {
    let path = report_path.with_extension("report.json");
    std::fs::write(&path, serde_json::to_vec_pretty(sidecar)?)
        .with_context(|| format!("写 sidecar 失败: {}", path.display()))?;
    Ok(())
}

async fn run_one(candidate: Candidate, timeout: Duration, reports_dir: PathBuf) -> Sidecar {
    let started = Instant::now();
    let future = stock_analysis::deep_analyzer::run_and_save(&candidate.code);
    let outcome = tokio::time::timeout(timeout, future).await;
    let elapsed_secs = started.elapsed().as_secs_f64();
    let (status, report_path, composite, error) = match outcome {
        Ok(Ok(path)) => {
            let composite = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| parse_composite(&text));
            ("ok".to_string(), Some(path.display().to_string()), composite, None)
        }
        Ok(Err(err)) => ("error".to_string(), None, None, Some(err.to_string())),
        Err(_) => ("timeout".to_string(), None, None, None),
    };
    let _ = reports_dir;
    let sidecar = Sidecar {
        code: candidate.code,
        name: candidate.name,
        source: candidate.source.to_string(),
        status,
        elapsed_secs,
        report_path: report_path.clone(),
        composite,
        error,
    };
    if let Some(raw) = report_path {
        let _ = write_sidecar(Path::new(&raw), &sidecar);
    }
    sidecar
}

fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|arg| arg == flag);

    let db_path = args
        .iter()
        .position(|arg| arg == "--db")
        .and_then(|index| args.get(index + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/stock_analysis.db"));
    let business_date = args
        .iter()
        .position(|arg| arg == "--date")
        .and_then(|index| args.get(index + 1))
        .cloned()
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
    let proposal_limit = args
        .iter()
        .position(|arg| arg == "--proposal-limit")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(3);
    let max = args
        .iter()
        .position(|arg| arg == "--max")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(9);
    let concurrency = args
        .iter()
        .position(|arg| arg == "--concurrency")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(2)
        .max(1);
    let timeout_secs = args
        .iter()
        .position(|arg| arg == "--timeout-secs")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(90);
    let budget_secs = args
        .iter()
        .position(|arg| arg == "--budget-secs")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1200);

    let candidates = select_candidates(&db_path, &business_date, proposal_limit, max)?;
    if candidates.is_empty() {
        println!("没有可分析标的（持仓 {db_path:?}，日期 {business_date}）");
        return Ok(());
    }

    if has("--dry-run") {
        let selected = Selected {
            business_date: business_date.clone(),
            candidates: candidates
                .iter()
                .map(|candidate| SelectedRow {
                    code: candidate.code.clone(),
                    name: candidate.name.clone(),
                    source: candidate.source.to_string(),
                })
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&selected)?);
        return Ok(());
    }

    let reports_dir = PathBuf::from("reports/details");
    std::fs::create_dir_all(&reports_dir)?;
    let probe_db = PathBuf::from("data/deep_analyze_batch_probe.db");
    std::env::set_var("DATABASE_PATH", &probe_db);
    stock_analysis::database::DatabaseManager::init(Some(probe_db))
        .map_err(|error| anyhow::anyhow!("probe DB 初始化失败: {error}"))?;

    let started_at = chrono::Local::now().to_rfc3339();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(concurrency.max(2))
        .enable_all()
        .build()
        .expect("tokio runtime");
    let selected_count = candidates.len();
    let results = runtime.block_on(async {
        let tasks = stream::iter(candidates.into_iter())
            .map(|candidate| run_one(candidate, Duration::from_secs(timeout_secs), reports_dir.clone()))
            .buffer_unordered(concurrency);
        tokio::time::timeout(Duration::from_secs(budget_secs), tasks.collect::<Vec<_>>()).await
    });
    let (budget_exceeded, results) = match results {
        Ok(results) => (false, results),
        Err(_) => (true, Vec::new()),
    };

    let summary = BatchSummary {
        business_date: business_date.clone(),
        started_at,
        finished_at: chrono::Local::now().to_rfc3339(),
        selected: selected_count,
        concurrency,
        timeout_secs,
        budget_exceeded,
        results,
    };
    let summary_path = reports_dir.join(format!("batch_{business_date}.json"));
    std::fs::write(&summary_path, serde_json::to_vec_pretty(&summary)?)?;

    for result in &summary.results {
        println!(
            "{} {} {} {:.1}s{}",
            result.code,
            result.name,
            result.status,
            result.elapsed_secs,
            result
                .composite
                .map(|value| format!(" composite={value}"))
                .unwrap_or_default()
        );
    }
    println!(
        "batch selected={} ok={} error={} timeout={} budget_exceeded={} → {}",
        summary.selected,
        summary.results.iter().filter(|r| r.status == "ok").count(),
        summary.results.iter().filter(|r| r.status == "error").count(),
        summary.results.iter().filter(|r| r.status == "timeout").count(),
        summary.budget_exceeded,
        summary_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_composite;

    #[test]
    fn parses_composite_from_generated_report() {
        assert_eq!(
            parse_composite("- 方向：减持（composite 41 落入 30–44 档）"),
            Some(41)
        );
        assert_eq!(parse_composite("- **加权综合：41/100**"), Some(41));
        assert_eq!(parse_composite("无评分"), None);
    }
}
