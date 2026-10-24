//! 通达信盘后日线文件（vipdoc/*/lday/*.day）导入 stock_daily.
//!
//! 为什么用这个源：盘后文件是本地定长记录，全历史、自带成交额（腾讯/新浪的日线接口
//! 都没有 amount），不受 provider 协议与限流影响；复盘/回测用它比任何 HTTP 源都稳。
//!
//! 用法:
//!   STOCK_DB=<db> tdx_day_import --dir ~/tdx_lday --since 2025-01-01
//!   STOCK_DB=<db> tdx_day_import --dir ~/tdx_lday --since 2025-01-01 --codes 600000,000001
//!   STOCK_DB=<db> tdx_day_import --dir ~/tdx_lday --since 2025-01-01 --dry-run
//!
//! 记录格式（32 字节，小端）: date u32(YYYYMMDD), open u32(分), high, low, close,
//! amount f32(元), volume u32(股), reserved u32。落库时价格 /100、成交量 /100（股→手，
//! 与 stock_daily 既有口径一致），amount 原样。
//!
//! 只收 A 股：sh6*（60/68），sz00*/sz30*；指数（sh000/sh880/sz399）与 B 股（sh9*/sz2*）跳过。
//! `pct_chg` 用文件内前一根收盘算，首根写 NULL。`ma5/10/20`、`volume_ratio` 不动（15:55 任务重算）。

use rusqlite::{params, Connection, OpenFlags};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

const RECORD: usize = 32;

fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(|value| value.as_str())
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn collect_day_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_day_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("day") {
            out.push(path);
        }
    }
}

/// 文件名 → 6 位 A 股代码；指数 / B 股 / 板块文件返回 None。
fn code_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    if stem.len() != 8 {
        return None;
    }
    let (market, code) = stem.split_at(2);
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    match market {
        "sh" if code.starts_with('6') => Some(code.to_string()),
        "sz" if code.starts_with("00") || code.starts_with("30") => Some(code.to_string()),
        _ => None,
    }
}

struct Bar {
    date: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    amount: f64,
    volume_lots: f64,
    pct_chg: Option<f64>,
}

fn parse_file(bytes: &[u8], since: &str) -> Vec<Bar> {
    let count = bytes.len() / RECORD;
    let mut bars = Vec::with_capacity(count);
    let mut prev_close: Option<f64> = None;
    for index in 0..count {
        let raw = &bytes[index * RECORD..(index + 1) * RECORD];
        let date_raw = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        let open = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as f64 / 100.0;
        let high = u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]) as f64 / 100.0;
        let low = u32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]) as f64 / 100.0;
        let close = u32::from_le_bytes([raw[16], raw[17], raw[18], raw[19]]) as f64 / 100.0;
        let amount = f32::from_le_bytes([raw[20], raw[21], raw[22], raw[23]]) as f64;
        let volume = u32::from_le_bytes([raw[24], raw[25], raw[26], raw[27]]) as f64;

        let year = date_raw / 10_000;
        if !(1990..=2100).contains(&year) || close <= 0.0 {
            continue;
        }
        let date = format!(
            "{:04}-{:02}-{:02}",
            year,
            (date_raw / 100) % 100,
            date_raw % 100
        );
        let pct_chg = prev_close.map(|previous| {
            if previous > 0.0 {
                (close / previous - 1.0) * 100.0
            } else {
                0.0
            }
        });
        prev_close = Some(close);
        if date.as_str() < since {
            continue;
        }
        bars.push(Bar {
            date,
            open,
            high,
            low,
            close,
            amount,
            volume_lots: volume / 100.0,
            pct_chg,
        });
    }
    bars
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("tdx_day_import --dir <vipdoc 根或解压目录> --since YYYY-MM-DD [--codes a,b] [--dry-run]");
        return;
    }
    let dir = expand_home(arg_value(&args, "--dir").unwrap_or("~/tdx_lday"));
    let since = arg_value(&args, "--since").unwrap_or("2025-01-01").to_string();
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let code_filter: Option<BTreeSet<String>> = arg_value(&args, "--codes").map(|raw| {
        raw.split(',')
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect()
    });

    let db_path = std::env::var("STOCK_DB").unwrap_or_else(|_| "data/stock_analysis.db".to_string());
    if !Path::new(&db_path).exists() {
        eprintln!("STOCK_DB 不存在: {db_path}");
        std::process::exit(2);
    }

    let mut files = Vec::new();
    collect_day_files(&dir, &mut files);
    files.sort();
    println!(
        "[盘后日线] dir={} files={} since={} dry_run={} codes={}",
        dir.display(),
        files.len(),
        since,
        dry_run,
        code_filter.as_ref().map(|s| s.len()).unwrap_or(0)
    );

    let mut conn = if dry_run {
        Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY).expect("打开只读库")
    } else {
        Connection::open(&db_path).expect("打开库")
    };
    conn.busy_timeout(std::time::Duration::from_secs(30))
        .expect("busy_timeout");
    let started = Instant::now();
    let mut file_count = 0usize;
    let mut skipped_files = 0usize;
    let mut bar_rows = 0usize;
    let mut latest: Option<String> = None;
    let mut earliest: Option<String> = None;
    let mut errors = 0usize;

    for path in &files {
        let Some(code) = code_from_path(path) else {
            skipped_files += 1;
            continue;
        };
        if let Some(filter) = &code_filter {
            if !filter.contains(&code) {
                continue;
            }
        }
        let Ok(bytes) = std::fs::read(path) else {
            errors += 1;
            continue;
        };
        let bars = parse_file(&bytes, &since);
        if bars.is_empty() {
            continue;
        }
        file_count += 1;
        if earliest.as_deref().map(|e| bars[0].date.as_str() < e).unwrap_or(true) {
            earliest = Some(bars[0].date.clone());
        }
        if latest.as_deref().map(|l| bars[bars.len() - 1].date.as_str() > l).unwrap_or(true) {
            latest = Some(bars[bars.len() - 1].date.clone());
        }
        if !dry_run {
            let tx = conn.transaction().expect("开事务");
            {
                let mut stmt = tx
                    .prepare(
                        "INSERT INTO stock_daily (code, date, open, high, low, close, volume, amount, pct_chg, data_source)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'tdx_day')
                         ON CONFLICT(code, date) DO UPDATE SET
                           open=excluded.open, high=excluded.high, low=excluded.low, close=excluded.close,
                           volume=excluded.volume, amount=excluded.amount, pct_chg=excluded.pct_chg,
                           data_source='tdx_day', updated_at=CURRENT_TIMESTAMP",
                    )
                    .expect("准备语句");
                for bar in &bars {
                    if let Err(error) = stmt.execute(params![
                        code,
                        bar.date,
                        bar.open,
                        bar.high,
                        bar.low,
                        bar.close,
                        bar.volume_lots,
                        bar.amount,
                        bar.pct_chg
                    ]) {
                        errors += 1;
                        if errors <= 5 {
                            eprintln!("! {code} {}: {error}", bar.date);
                        }
                    } else {
                        bar_rows += 1;
                    }
                }
            }
            tx.commit().expect("提交");
        } else {
            bar_rows += bars.len();
        }
        if file_count % 500 == 0 {
            println!(
                "[盘后日线] files={file_count} bars={bar_rows} errors={errors} elapsed={:?}",
                started.elapsed()
            );
        }
    }

    println!(
        "[盘后日线] 完成 files={file_count} rows={bar_rows} skipped_non_a={skipped_files} errors={errors} range={}..{} elapsed={:?}",
        earliest.unwrap_or_else(|| "-".into()),
        latest.unwrap_or_else(|| "-".into()),
        started.elapsed()
    );
}
