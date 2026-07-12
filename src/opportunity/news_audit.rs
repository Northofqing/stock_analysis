// -*- coding: utf-8 -*-
//! 新闻 Ranker 审计日志 (P2-News Commit 5)
//!
//! **目的**: 落盘每条 RankedNews 到 `data/news_rank_audit_YYYY-MM-DD.jsonl`,
//!   供后续 D+1/D+3/D+5 回看 (MFE/MAE/可执行买点, 不自动调权)
//!
//! **触发**: `NEWS_RANK_AUDIT=true` env var (默认 false, 不刷盘)
//!
//! **红线**:
//!   - 写入失败仅 warn, 不 panic
//!   - 路径走 DATABASE_PATH 同目录, 不硬编码
//!   - 追加模式 (不清旧, 不覆盖)
//!   - 单行失败不影响其他行
//!
//! **字段** (人读 + 后续脚本可读):
//!   ts, candidate_id, title, source, chain, board_code,
//!   event_type, heat_stage, score, bucket,
//!   rule_score, freshness_score, heat_score, stage_score,
//!   capital_score, source_score, risk_penalty,
//!   reasons, drop_reason
use crate::opportunity::news_ranker::{HeatStage, NewsRankBucket, RankedNews};
use chrono::Local;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// 审计单行 (扁平 JSON, 后续回看脚本友好)
#[derive(Debug, Serialize)]
struct AuditRow {
    ts: String,
    candidate_id: String,
    title: String,
    source: String,
    chain: String,
    board_code: Option<String>,
    event_type: String,
    heat_stage: String,
    score: i32,
    bucket: String,
    rule_score: i32,
    freshness_score: i32,
    heat_score: i32,
    stage_score: i32,
    capital_score: i32,
    source_score: i32,
    risk_penalty: i32,
    reasons: Vec<String>,
    drop_reason: Option<String>,
}

impl AuditRow {
    fn from_ranked(r: &RankedNews) -> Self {
        let chain = r
            .candidate
            .chain_hits
            .first()
            .map(|h| h.chain.clone())
            .unwrap_or_default();
        Self {
            ts: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            candidate_id: r.candidate.id.clone(),
            title: r.candidate.title.clone(),
            source: r.candidate.source.clone(),
            chain,
            board_code: r.candidate.board_code.clone(),
            event_type: r.event_type.label().to_string(),
            heat_stage: heat_stage_label(r.heat_stage),
            score: r.score,
            bucket: bucket_label(r.bucket),
            rule_score: r.evidence.rule_score,
            freshness_score: r.evidence.freshness_score,
            heat_score: r.evidence.heat_score,
            stage_score: r.evidence.stage_score,
            capital_score: r.evidence.capital_score,
            source_score: r.evidence.source_score,
            risk_penalty: r.evidence.risk_penalty,
            reasons: r.reasons.clone(),
            drop_reason: r.drop_reason.clone(),
        }
    }
}

fn heat_stage_label(s: HeatStage) -> String {
    s.label().to_string()
}

fn bucket_label(b: NewsRankBucket) -> String {
    match b {
        NewsRankBucket::PushNow => "PushNow".to_string(),
        NewsRankBucket::WatchCandidate => "WatchCandidate".to_string(),
        NewsRankBucket::LogOnly => "LogOnly".to_string(),
        NewsRankBucket::Drop => "Drop".to_string(),
    }
}

/// 审计路径: DATABASE_PATH 同目录优先, 兜底 ./data/
pub fn audit_path() -> PathBuf {
    let dir = std::env::var("DATABASE_PATH")
        .ok()
        .and_then(|p| {
            let pb = PathBuf::from(p);
            pb.parent().map(|p| p.to_path_buf())
        })
        .unwrap_or_else(|| PathBuf::from("./data"));
    let today = Local::now().format("%Y-%m-%d").to_string();
    dir.join(format!("news_rank_audit_{}.jsonl", today))
}

/// 追加 ranked 列表到 path (显式 path, 供 test 隔离)
///
/// **红线**:
///   - 路径不存在 → 自动创建父目录
///   - 写失败 → warn, 不 panic
///   - 单行失败 → 跳过, 不影响其他
pub fn write_audit_jsonl_at(ranked: &[RankedNews], path: &Path) -> usize {
    if ranked.is_empty() {
        return 0;
    }
    if let Err(e) = ensure_parent_dir(path) {
        log::warn!("[NEWS_AUDIT] 创建父目录失败: {:#}", e);
        return 0;
    }
    let mut file = match OpenOptions::new().create(true).append(true).open(path) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("[NEWS_AUDIT] 打开 {} 失败: {:#}", path.display(), e);
            return 0;
        }
    };
    let mut written = 0;
    for r in ranked {
        let row = AuditRow::from_ranked(r);
        match serde_json::to_string(&row) {
            Ok(line) => {
                if let Err(e) = writeln!(file, "{}", line) {
                    log::warn!("[NEWS_AUDIT] 写行失败: {:#}", e);
                } else {
                    written += 1;
                }
            }
            Err(e) => log::warn!("[NEWS_AUDIT] 序列化失败: {:#}", e),
        }
    }
    log::info!("[NEWS_AUDIT] 落 {} 条到 {}", written, path.display());
    written
}

/// 追加 ranked 列表到当日 JSONL (env 控开关 + 走默认 path)
///
/// **触发**: `NEWS_RANK_AUDIT=true` env var
/// **不触发**: 直接返 0, 不读盘
pub fn write_audit_jsonl(ranked: &[RankedNews]) -> usize {
    let enabled = std::env::var("NEWS_RANK_AUDIT")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);
    if !enabled {
        return 0;
    }
    write_audit_jsonl_at(ranked, &audit_path())
}

fn ensure_parent_dir(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opportunity::chain_mapper::{ChainHit, ChainSource};
    use crate::opportunity::news_ranker::{
        EventType, NewsCandidate, NewsEvidenceBreakdown, RankedNews,
    };
    use chrono::Local;
    use std::env;
    use std::sync::Mutex;

    /// env var 并行 race condition 串行化 (cargo test 多线程)
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn unique_tmp(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        env::temp_dir().join(format!(
            "news_audit_{}_{}_{}_{}.jsonl",
            tag,
            std::process::id(),
            nanos,
            n
        ))
    }

    fn mock_ranked(id: &str, title: &str, bucket: NewsRankBucket, score: i32) -> RankedNews {
        RankedNews {
            candidate: NewsCandidate {
                id: id.to_string(),
                title: title.to_string(),
                snippet: "".into(),
                source: "test".into(),
                published_at: Some(Local::now()),
                chain_hits: vec![ChainHit {
                    chain: "测试链".into(),
                    keywords: vec!["测试".into()],
                    logic: "test".into(),
                    stocks: vec![],
                    source: ChainSource::Rule,
                    board_keyword: "测试".into(),
                    fund_flow_pct: None,
                    board_code: None,
                    board_change_pct: None,
                }],
                board_code: None,
            },
            event_type: EventType::PolicyCatalyst,
            heat_stage: crate::opportunity::news_ranker::HeatStage::Start,
            score,
            bucket,
            evidence: NewsEvidenceBreakdown {
                rule_score: 20,
                freshness_score: 15,
                heat_score: 8,
                stage_score: 25,
                capital_score: 8,
                source_score: 10,
                risk_penalty: 0,
            },
            reasons: vec!["规则 20".into()],
            drop_reason: None,
        }
    }

    /// 1) env=false → 不写盘, 返 0 (串行化避免并行 env 污染)
    #[test]
    fn disabled_no_op() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        env::set_var("NEWS_RANK_AUDIT", "false");
        let r = vec![mock_ranked(
            "t1",
            "国务院印发规划",
            crate::opportunity::news_ranker::NewsRankBucket::PushNow,
            75,
        )];
        assert_eq!(write_audit_jsonl(&r), 0);
        env::remove_var("NEWS_RANK_AUDIT");
    }

    /// 2) env=true + 空 ranked → 返 0
    #[test]
    fn enabled_empty_returns_zero() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        env::set_var("NEWS_RANK_AUDIT", "true");
        assert_eq!(write_audit_jsonl(&[]), 0);
        env::remove_var("NEWS_RANK_AUDIT");
    }

    /// 3) ranked → 落盘 + 行数对 (用 _at 显式 path 避免并行 env 污染)
    #[test]
    fn enabled_writes_lines() {
        let path = unique_tmp("audit");
        let _ = std::fs::remove_file(&path);

        let r = vec![
            mock_ranked(
                "a",
                "国务院印发低空经济规划",
                crate::opportunity::news_ranker::NewsRankBucket::PushNow,
                75,
            ),
            mock_ranked(
                "b",
                "证监会立案调查某公司",
                crate::opportunity::news_ranker::NewsRankBucket::Drop,
                10,
            ),
        ];
        let n = write_audit_jsonl_at(&r, &path);
        assert_eq!(n, 2, "应落 2 条");

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "文件应 2 行");
        assert!(
            lines[0].contains("PushNow"),
            "第一行应 PushNow: {}",
            lines[0]
        );
        assert!(lines[1].contains("Drop"), "第二行应 Drop: {}", lines[1]);

        let _ = std::fs::remove_file(&path);
    }

    /// 3.5) 默认 write_audit_jsonl 在 NEWS_RANK_AUDIT=false 时不写 (强制清 env)
    #[test]
    fn default_disabled() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        env::set_var("NEWS_RANK_AUDIT", "false");
        let path = unique_tmp("default");
        let _ = std::fs::remove_file(&path);
        let r = vec![mock_ranked(
            "x",
            "test",
            crate::opportunity::news_ranker::NewsRankBucket::LogOnly,
            30,
        )];
        let n = write_audit_jsonl(&r);
        assert_eq!(n, 0, "env=false 应不写");
        assert!(!path.exists(), "默认路径不应被写");
        env::remove_var("NEWS_RANK_AUDIT");
    }

    /// 3.6) 默认 write_audit_jsonl 在 NEWS_RANK_AUDIT=true 时写 audit_path
    #[test]
    fn default_enabled_writes_to_audit_path() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = env::var("NEWS_RANK_AUDIT").ok();
        let prev_db = env::var("DATABASE_PATH").ok();
        let tmp_db = unique_tmp("db_default");
        let _ = std::fs::remove_file(&tmp_db);
        env::set_var("DATABASE_PATH", &tmp_db);
        env::set_var("NEWS_RANK_AUDIT", "true");

        let r = vec![mock_ranked(
            "y",
            "test",
            crate::opportunity::news_ranker::NewsRankBucket::PushNow,
            70,
        )];
        let n = write_audit_jsonl(&r);
        assert_eq!(n, 1, "应落 1 条");

        let path = audit_path();
        assert!(path.exists(), "audit_path 应存在");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("PushNow"));

        // 清理
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&tmp_db);
        match prev {
            Some(v) => env::set_var("NEWS_RANK_AUDIT", v),
            None => env::remove_var("NEWS_RANK_AUDIT"),
        }
        match prev_db {
            Some(v) => env::set_var("DATABASE_PATH", v),
            None => env::remove_var("DATABASE_PATH"),
        }
    }

    /// 4) audit_path 走 DATABASE_PATH 同目录
    #[test]
    fn audit_path_uses_database_dir() {
        let prev = env::var("DATABASE_PATH").ok();
        env::set_var("DATABASE_PATH", "/tmp/foo/bar.db");
        let p = audit_path();
        assert!(p.to_string_lossy().contains("/tmp/foo"));
        assert!(p.to_string_lossy().contains("news_rank_audit_"));
        match prev {
            Some(v) => env::set_var("DATABASE_PATH", v),
            None => env::remove_var("DATABASE_PATH"),
        }
    }
}
