// -*- coding: utf-8 -*-
//! 板块 N 日历史缓存 (P2-News Commit 0)
//!
//! **目的**: 给 detect_heat_stage (P2-News Commit 2) 提供"近 3 日累计涨幅"等
//! N 日历史数据, 单日 `change_pct` 区分不了"今天刚启动" vs "已涨 3 天".
//!
//! **存储**: `data/sector_history.jsonl` (JSON Lines, 追加写, 与 DB 路径独立)
//!   - 路径可由 `SECTOR_HISTORY_PATH` 环境变量覆盖
//!   - 留 30 天滚动, 超出 cleanup 删除
//!
//! **写入触发**: 在 `fetch_board_ranking` 后, 由 main.rs 调 `append_today(&boards)`
//!   (保持 fetch 函数纯, 副作用外置)
//!
//! **红线**:
//!   - 任一读写/解析失败 → 显式 `Err`，调用方不得当作空历史
//!   - 文件不存在 → 作为尚无历史，返回空批次
//!   - 同一 (code, date) 重复 append → 去重 (后写覆盖前写)
//!   - BR-117 仅允许需要三日证据的新闻阶段读取本历史；坏批次不得补零
//!
//! **API**: 所有读写函数都有 `*_at(path)` 显式 path 形式 (供 test 隔离),
//!   无 `_at` 形式从 `SECTOR_HISTORY_PATH` 兜底 `./data/sector_history.jsonl`.
use crate::market_analyzer::sector_monitor::ConceptBoard;
use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// 单条板块历史记录 (1 board × 1 day)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BoardDay {
    pub code: String,
    pub name: String,
    pub date: NaiveDate,
    pub change_pct: f64,
    pub main_inflow: f64,
    pub main_net_pct_today: f64,
    pub main_net_pct_5d: f64,
    pub vol_ratio: f64,
    pub turnover: f64,
}

impl BoardDay {
    /// 从 ConceptBoard 派生 (date 用今天)
    pub fn from_concept_board(b: &ConceptBoard) -> Self {
        Self {
            code: b.code.clone(),
            name: b.name.clone(),
            date: Local::now().date_naive(),
            change_pct: b.change_pct,
            main_inflow: b.main_inflow,
            main_net_pct_today: b.main_net_pct_today,
            main_net_pct_5d: b.main_net_pct_5d,
            vol_ratio: b.vol_ratio,
            turnover: b.turnover,
        }
    }
}

/// 默认路径: SECTOR_HISTORY_PATH 优先, 兜底 data/sector_history.jsonl
pub fn history_path() -> PathBuf {
    std::env::var("SECTOR_HISTORY_PATH")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./data/sector_history.jsonl"))
}

// ============ *_at(path) 形式 — 显式 path, 供 test 隔离用 ============

/// 加载历史；任一非空坏行拒绝整批。
pub fn load_history_at(path: &Path) -> Result<Vec<BoardDay>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file =
        fs::File::open(path).with_context(|| format!("打开 sector_history {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("读取 sector_history 第 {} 行", i + 1))?;
        if line.trim().is_empty() {
            anyhow::bail!("sector_history 第 {} 行为空", i + 1);
        }
        let board: BoardDay = serde_json::from_str(&line)
            .with_context(|| format!("解析 sector_history 第 {} 行", i + 1))?;
        let numerics = [
            board.change_pct,
            board.main_inflow,
            board.main_net_pct_today,
            board.main_net_pct_5d,
            board.vol_ratio,
            board.turnover,
        ];
        if board.code.trim().is_empty()
            || board.name.trim().is_empty()
            || numerics.iter().any(|value| !value.is_finite())
            || board.change_pct.abs() > 20.0
            || board.main_net_pct_today.abs() > 100.0
            || board.main_net_pct_5d.abs() > 100.0
            || board.vol_ratio < 0.0
            || board.turnover < 0.0
        {
            anyhow::bail!("sector_history 第 {} 行字段非法", i + 1);
        }
        out.push(board);
    }
    validate_history_continuity(&out)?;
    Ok(out)
}

fn validate_history_continuity(rows: &[BoardDay]) -> Result<()> {
    let mut by_code: std::collections::HashMap<&str, Vec<NaiveDate>> =
        std::collections::HashMap::new();
    for row in rows {
        by_code.entry(&row.code).or_default().push(row.date);
    }
    for (code, dates) in &mut by_code {
        dates.sort_unstable();
        for pair in dates.windows(2) {
            if pair[0] == pair[1] {
                anyhow::bail!("sector_history {code} 日期重复: {}", pair[0]);
            }
            let expected = crate::calendar::next_trading_day(pair[0]);
            if pair[1] != expected {
                anyhow::bail!(
                    "sector_history {code} 交易日断档: {} 后应为 {}, 实际为 {}",
                    pair[0],
                    expected,
                    pair[1]
                );
            }
        }
    }
    Ok(())
}

/// 追加今日 boards 到 path (按 (code, date) 去重 — 同日同板覆盖)
pub fn append_today_at(boards: &[ConceptBoard], path: &Path) -> Result<usize> {
    if boards.is_empty() {
        return Ok(0);
    }
    ensure_parent_dir(path).context("创建 sector_history 父目录失败")?;

    let today = Local::now().date_naive();
    let mut existing = load_history_at(path)?;
    let before = existing.len();
    existing.retain(|b| !(b.date == today && boards.iter().any(|nb| nb.code == b.code)));
    let replaced = before.saturating_sub(existing.len());

    for b in boards {
        existing.push(BoardDay::from_concept_board(b));
    }

    let tmp = path.with_extension("jsonl.tmp");
    write_jsonl(&tmp, &existing).context("写 sector_history.tmp 失败")?;
    fs::rename(&tmp, path).context("rename sector_history 失败")?;

    log::info!(
        "[SECTOR_HISTORY] 今日追加 {} 条 (覆盖同日旧值 {} 条, 累计 {} 条)",
        boards.len(),
        replaced,
        existing.len()
    );
    Ok(boards.len())
}

/// 取某板块最近 N 日数据 (按 date 降序)
pub fn fetch_board_history_at(code: &str, n: usize, path: &Path) -> Result<Vec<BoardDay>> {
    let mut all: Vec<BoardDay> = load_history_at(path)?
        .into_iter()
        .filter(|b| b.code == code)
        .collect();
    all.sort_by_key(|item| std::cmp::Reverse(item.date));
    Ok(all.into_iter().take(n).collect())
}

/// 删 retention_days 之前的数据, 返删除条数
pub fn cleanup_at(retention_days: usize, path: &Path) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let cutoff = Local::now().date_naive() - chrono::Duration::days(retention_days as i64);
    let mut all = load_history_at(path)?;
    let before = all.len();
    all.retain(|b| b.date >= cutoff);
    let removed = before - all.len();
    if removed == 0 {
        return Ok(0);
    }
    let tmp = path.with_extension("jsonl.tmp");
    write_jsonl(&tmp, &all)?;
    fs::rename(&tmp, path)?;
    log::info!(
        "[SECTOR_HISTORY] cleanup 删 {} 条 (< {}), 剩 {} 条",
        removed,
        cutoff,
        all.len()
    );
    Ok(removed)
}

// ============ 默认 (走 env) 形式 — 生产路径 ============

pub fn load_history() -> Result<Vec<BoardDay>> {
    load_history_at(&history_path())
}

pub fn append_today(boards: &[ConceptBoard]) -> Result<usize> {
    append_today_at(boards, &history_path())
}

pub fn fetch_board_history(code: &str, n: usize) -> Result<Vec<BoardDay>> {
    fetch_board_history_at(code, n, &history_path())
}

pub fn cleanup(retention_days: usize) -> Result<usize> {
    cleanup_at(retention_days, &history_path())
}

// ============ 派生函数 (供后续 detect_heat_stage 用) ============

/// 板块 N 日累计涨幅 (近 n_days 涨幅相加, 含今日) — 显式 path 形式
pub fn cumulative_change_pct_at(code: &str, n_days: usize, path: &Path) -> Result<Option<f64>> {
    let history = fetch_board_history_at(code, n_days, path)?;
    if history.len() != n_days {
        return Ok(None);
    }
    Ok(Some(history.iter().map(|b| b.change_pct).sum()))
}

/// 板块 N 日累计涨幅 — 默认 path 形式 (走 history_path)
pub fn cumulative_change_pct(code: &str, n_days: usize) -> Result<Option<f64>> {
    cumulative_change_pct_at(code, n_days, &history_path())
}

/// 板块 N 日平均资金加速度 (今日 - 5日均, 取近 n_days 平均) — 显式 path
pub fn avg_inflow_accel_at(code: &str, n_days: usize, path: &Path) -> Result<Option<f64>> {
    let history = fetch_board_history_at(code, n_days, path)?;
    if history.len() != n_days {
        return Ok(None);
    }
    let sum: f64 = history
        .iter()
        .map(|b| b.main_net_pct_today - b.main_net_pct_5d)
        .sum();
    Ok(Some(sum / history.len() as f64))
}

/// 板块 N 日平均资金加速度 — 默认 path
pub fn avg_inflow_accel(code: &str, n_days: usize) -> Result<Option<f64>> {
    avg_inflow_accel_at(code, n_days, &history_path())
}

// ============ helpers ============

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建父目录 {}", parent.display()))?;
        }
    }
    Ok(())
}

fn write_jsonl(path: &Path, items: &[BoardDay]) -> Result<()> {
    let mut f = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("打开 {}", path.display()))?;
    for b in items {
        let line = serde_json::to_string(b).context("BoardDay 序列化失败")?;
        writeln!(f, "{}", line).context("写 jsonl 行失败")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// 唯一 tmp path (test 内调用, 避免并行 env 污染)
    fn unique_tmp(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        env::temp_dir().join(format!(
            "sector_history_{}_{}_{}_{}.jsonl",
            tag, pid, nanos, n
        ))
    }

    fn mock_board(code: &str, name: &str, chg: f64) -> ConceptBoard {
        ConceptBoard {
            code: code.to_string(),
            name: name.to_string(),
            change_pct: chg,
            main_inflow: 1e8,
            leader_name: "龙头".to_string(),
            vol_ratio: 1.5,
            turnover: 3.0,
            main_net_pct_today: 5.0,
            main_net_pct_5d: 2.0,
        }
    }

    /// 1) 写 1 条 → load 读回 1 条
    #[test]
    fn append_and_load_roundtrip() {
        let path = unique_tmp("roundtrip");
        let _ = fs::remove_file(&path);
        let boards = vec![mock_board("BK0001", "测试板块", 3.5)];
        append_today_at(&boards, &path).unwrap();
        let all = load_history_at(&path).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].code, "BK0001");
        assert_eq!(all[0].change_pct, 3.5);
        assert_eq!(all[0].name, "测试板块");
        let _ = fs::remove_file(&path);
    }

    /// 2) 同日同 code 多次 append → 去重覆盖
    #[test]
    fn append_dedup_same_day_same_code() {
        let path = unique_tmp("dedup");
        let _ = fs::remove_file(&path);
        let v1 = vec![mock_board("BK0001", "测试板块", 1.0)];
        let v2 = vec![mock_board("BK0001", "测试板块", 5.0)];
        append_today_at(&v1, &path).unwrap();
        append_today_at(&v2, &path).unwrap();
        let all = load_history_at(&path).unwrap();
        assert_eq!(all.len(), 1, "同日同 code 不应重复");
        assert_eq!(all[0].change_pct, 5.0, "后写覆盖前写");
        let _ = fs::remove_file(&path);
    }

    /// 3) 写入 2 个不同 code → load 返 2 条
    #[test]
    fn append_multi_codes() {
        let path = unique_tmp("multi");
        let _ = fs::remove_file(&path);
        let v = vec![
            mock_board("BK0001", "板块1", 1.0),
            mock_board("BK0002", "板块2", 2.0),
        ];
        append_today_at(&v, &path).unwrap();
        let all = load_history_at(&path).unwrap();
        assert_eq!(all.len(), 2);
        let _ = fs::remove_file(&path);
    }

    /// 4) 跨日: 手工写历史 (昨天 + 前天), 今天再写 3.0
    ///
    /// fetch_board_history 返 3 条 (降序, 今日 3.0 → 昨天 2.0 → 前天 1.0)
    #[test]
    fn fetch_history_returns_descending() {
        let path = unique_tmp("fetch");
        let _ = fs::remove_file(&path);
        let dates = [
            NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 16).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 17).unwrap(),
        ];
        let mut history = Vec::new();
        for (date, chg) in dates.into_iter().zip([1.0_f64, 2.0, 3.0]) {
            history.push(BoardDay {
                code: "BK0001".to_string(),
                name: "测试板块".to_string(),
                date,
                change_pct: chg,
                main_inflow: 1e8,
                main_net_pct_today: 3.0,
                main_net_pct_5d: 1.0,
                vol_ratio: 1.0,
                turnover: 2.0,
            });
        }
        ensure_parent_dir(&path).unwrap();
        write_jsonl(&path, &history).unwrap();

        let got = fetch_board_history_at("BK0001", 3, &path).unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].change_pct, 3.0, "今日排第 1");
        assert_eq!(got[1].change_pct, 2.0, "昨日排第 2");
        assert_eq!(got[2].change_pct, 1.0, "前日排第 3");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn br092_history_rejects_duplicate_and_missing_trading_days() {
        let row = |date| BoardDay {
            code: "BK_TEST".to_string(),
            name: "测试板块".to_string(),
            date,
            change_pct: 1.0,
            main_inflow: 1e8,
            main_net_pct_today: 2.0,
            main_net_pct_5d: 1.0,
            vol_ratio: 1.0,
            turnover: 2.0,
        };
        let first = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
        let gap = NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();
        assert!(validate_history_continuity(&[row(first), row(first)]).is_err());
        assert!(validate_history_continuity(&[row(first), row(gap)]).is_err());
    }

    /// 5) cleanup 删旧: 60 天前 1 条 + 今天 1 条, cleanup(30) → 删 1 条
    #[test]
    fn cleanup_removes_old() {
        let path = unique_tmp("cleanup");
        let _ = fs::remove_file(&path);
        let today = Local::now().date_naive();
        let old = BoardDay {
            code: "BK0001".to_string(),
            name: "老".to_string(),
            date: today - chrono::Duration::days(60),
            change_pct: 1.0,
            main_inflow: 0.0,
            main_net_pct_today: 0.0,
            main_net_pct_5d: 0.0,
            vol_ratio: 0.0,
            turnover: 0.0,
        };
        ensure_parent_dir(&path).unwrap();
        write_jsonl(&path, &[old]).unwrap();
        append_today_at(&[mock_board("BK0002", "今", 1.0)], &path).unwrap();

        let removed = cleanup_at(30, &path).unwrap();
        assert_eq!(removed, 1, "60 天前的应被删");
        let after = load_history_at(&path).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].code, "BK0002");
        let _ = fs::remove_file(&path);
    }

    /// 6) cumulative_change_pct: 3 日累计
    #[test]
    fn cumulative_change_basic() {
        let path = unique_tmp("cum");
        let _ = fs::remove_file(&path);
        let mut items = Vec::new();
        for (date, chg) in [
            (NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(), 3.0_f64),
            (NaiveDate::from_ymd_opt(2026, 7, 16).unwrap(), 2.0_f64),
            (NaiveDate::from_ymd_opt(2026, 7, 17).unwrap(), 1.0_f64),
        ] {
            items.push(BoardDay {
                code: "BK0001".to_string(),
                name: "X".to_string(),
                date,
                change_pct: chg,
                main_inflow: 0.0,
                main_net_pct_today: 0.0,
                main_net_pct_5d: 0.0,
                vol_ratio: 0.0,
                turnover: 0.0,
            });
        }
        ensure_parent_dir(&path).unwrap();
        write_jsonl(&path, &items).unwrap();

        let cum = cumulative_change_pct_at("BK0001", 3, &path)
            .unwrap()
            .unwrap_or(f64::NAN);
        assert!(
            (cum - 6.0).abs() < 1e-6,
            "3 日累计 = 1+2+3 = 6, got {}",
            cum
        );
        let _ = fs::remove_file(&path);
    }

    /// 7) 数据不足返 None
    #[test]
    fn cumulative_returns_none_when_empty() {
        let path = unique_tmp("none");
        let _ = fs::remove_file(&path);
        // 路径不存在, load_history_at 返空, cumulative 返 None
        assert!(cumulative_change_pct_at("BK0000", 3, &path)
            .unwrap()
            .is_none());
        assert!(avg_inflow_accel_at("BK0000", 3, &path).unwrap().is_none());
    }

    #[test]
    fn br117_cumulative_requires_the_complete_requested_window() {
        let path = unique_tmp("insufficient_window");
        let rows = [
            (NaiveDate::from_ymd_opt(2026, 7, 16).unwrap(), 1.0_f64),
            (NaiveDate::from_ymd_opt(2026, 7, 17).unwrap(), 2.0_f64),
        ]
        .into_iter()
        .map(|(date, change_pct)| BoardDay {
            code: "BK_TEST".to_string(),
            name: "测试板块".to_string(),
            date,
            change_pct,
            main_inflow: 1e8,
            main_net_pct_today: 2.0,
            main_net_pct_5d: 1.0,
            vol_ratio: 1.0,
            turnover: 2.0,
        })
        .collect::<Vec<_>>();
        ensure_parent_dir(&path).unwrap();
        write_jsonl(&path, &rows).unwrap();

        assert!(cumulative_change_pct_at("BK_TEST", 3, &path)
            .unwrap()
            .is_none());
        assert!(avg_inflow_accel_at("BK_TEST", 3, &path).unwrap().is_none());
        let _ = fs::remove_file(path);
    }

    /// 8) 任一解析失败行拒绝整批
    #[test]
    fn parse_error_rejects_entire_batch() {
        let path = unique_tmp("parse");
        let _ = fs::remove_file(&path);
        ensure_parent_dir(&path).unwrap();
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        writeln!(f, "not a json line").unwrap();
        writeln!(
            f,
            r#"{{"code":"BK0001","name":"X","date":"2026-07-01","change_pct":1.0,"main_inflow":0.0,"main_net_pct_today":0.0,"main_net_pct_5d":0.0,"vol_ratio":0.0,"turnover":0.0}}"#
        )
        .unwrap();
        drop(f);
        assert!(load_history_at(&path).is_err());
        let _ = fs::remove_file(&path);
    }
}
