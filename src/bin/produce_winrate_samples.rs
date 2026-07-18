//! BR-017: 从 canonical `prediction_tracker` 导出已验证胜率样本。
//!
//! 用法：
//! `cargo run --bin produce_winrate_samples -- --days 60 --n-day 5`

use chrono::{Duration, Local};
use diesel::prelude::*;
use serde::Serialize;
use std::path::PathBuf;
use stock_analysis::database::DatabaseManager;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    days: i64,
    n_day: u8,
    output: PathBuf,
}

fn parse_args(values: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut days = 60_i64;
    let mut n_day = 5_u8;
    let mut output = PathBuf::from("data/winrate_samples.jsonl");
    let args: Vec<String> = values.into_iter().collect();
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--days" => {
                index += 1;
                days = args
                    .get(index)
                    .ok_or("--days 缺少值")?
                    .parse::<i64>()
                    .map_err(|error| format!("--days 非法: {error}"))?;
            }
            "--n-day" => {
                index += 1;
                n_day = args
                    .get(index)
                    .ok_or("--n-day 缺少值")?
                    .parse::<u8>()
                    .map_err(|error| format!("--n-day 非法: {error}"))?;
            }
            "--output" => {
                index += 1;
                output = PathBuf::from(args.get(index).ok_or("--output 缺少值")?);
            }
            "--help" | "-h" => {
                return Err(
                    "usage: produce_winrate_samples [--days N] [--n-day 1|3|5] [--output PATH]"
                        .to_string(),
                );
            }
            unknown => return Err(format!("未知参数: {unknown}")),
        }
        index += 1;
    }
    if days <= 0 {
        return Err("--days 必须 > 0".to_string());
    }
    if !matches!(n_day, 1 | 3 | 5) {
        return Err("--n-day 只支持 1、3、5".to_string());
    }
    Ok(Args {
        days,
        n_day,
        output,
    })
}

#[derive(Debug, QueryableByName)]
struct VerifiedPredictionRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    pred_date: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    target_date: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    theme_name: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    stock_code: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pred_direction: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pred_score: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    reason: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Double)]
    actual_change: f64,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    hit: i32,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    special_case: Option<String>,
}

#[derive(Debug, Serialize)]
struct WinrateSample {
    pred_date: String,
    target_date: String,
    theme_name: Option<String>,
    stock_code: Option<String>,
    pred_direction: String,
    pred_score: Option<f64>,
    reason: Option<String>,
    n_day: u8,
    actual_change: f64,
    hit: bool,
    special_case: Option<String>,
}

fn load_samples(args: &Args) -> Result<Vec<WinrateSample>, Box<dyn std::error::Error>> {
    let db = DatabaseManager::get();
    let mut conn = db.get_conn()?;
    let since = (Local::now().date_naive() - Duration::days(args.days))
        .format("%Y-%m-%d")
        .to_string();
    let (change_column, hit_column, special_column) = match args.n_day {
        1 => ("actual_change_t1", "hit_t1", "t1_special_case"),
        3 => ("actual_change_t3", "hit_t3", "NULL"),
        5 => ("actual_change_t5", "hit_t5", "NULL"),
        _ => return Err("unsupported n_day".into()),
    };
    let sql = format!(
        "SELECT pred_date, target_date, theme_name, stock_code, pred_direction, pred_score, reason, \
         {change_column} AS actual_change, {hit_column} AS hit, {special_column} AS special_case \
         FROM prediction_tracker \
         WHERE pred_date >= ? AND {change_column} IS NOT NULL AND {hit_column} IS NOT NULL \
         ORDER BY pred_date, id"
    );
    let rows = diesel::sql_query(sql)
        .bind::<diesel::sql_types::Text, _>(since)
        .load::<VerifiedPredictionRow>(&mut conn)?;
    rows.into_iter()
        .map(|row| {
            if !row.actual_change.is_finite() || row.actual_change.abs() > 20.0 {
                return Err(format!(
                    "{} {:?} actual_change 非法: {}",
                    row.pred_date, row.stock_code, row.actual_change
                ));
            }
            if !matches!(row.hit, 0 | 1) {
                return Err(format!("{} hit 非法: {}", row.pred_date, row.hit));
            }
            if row
                .pred_score
                .is_some_and(|score| !score.is_finite() || !(0.0..=100.0).contains(&score))
            {
                return Err(format!("{} pred_score 非法", row.pred_date));
            }
            Ok(WinrateSample {
                pred_date: row.pred_date,
                target_date: row.target_date,
                theme_name: row.theme_name,
                stock_code: row.stock_code,
                pred_direction: row.pred_direction,
                pred_score: row.pred_score,
                reason: row.reason,
                n_day: args.n_day,
                actual_change: row.actual_change,
                hit: row.hit == 1,
                special_case: row.special_case,
            })
        })
        .collect::<Result<Vec<_>, String>>()
        .map_err(Into::into)
}

fn write_samples_atomic(
    output: &std::path::Path,
    samples: &[WinrateSample],
) -> Result<(), Box<dyn std::error::Error>> {
    if samples.is_empty() {
        return Err("没有已验证样本，拒绝写空占位文件".into());
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = output.with_extension("jsonl.tmp");
    let mut text = String::new();
    for sample in samples {
        text.push_str(&serde_json::to_string(sample)?);
        text.push('\n');
    }
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, output)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args(std::env::args()).map_err(|error| format!("参数错误: {error}"))?;
    let db_path = std::env::var("STOCK_DB").ok().map(PathBuf::from);
    DatabaseManager::init(db_path).map_err(|error| format!("数据库初始化失败: {error}"))?;
    let samples = load_samples(&args)?;
    write_samples_atomic(&args.output, &samples)?;
    println!(
        "已从 prediction_tracker 导出 {} 条 T{} 已验证样本到 {}",
        samples.len(),
        args.n_day,
        args.output.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_documented_flags() {
        let args = parse_args([
            "bin".to_string(),
            "--days".to_string(),
            "30".to_string(),
            "--n-day".to_string(),
            "3".to_string(),
            "--output".to_string(),
            "/tmp/samples.jsonl".to_string(),
        ])
        .expect("valid args");
        assert_eq!(args.days, 30);
        assert_eq!(args.n_day, 3);
        assert_eq!(args.output, PathBuf::from("/tmp/samples.jsonl"));
    }

    #[test]
    fn rejects_unsupported_horizon() {
        let error = parse_args(["bin".to_string(), "--n-day".to_string(), "2".to_string()])
            .expect_err("unsupported horizon");
        assert!(error.contains("只支持"));
    }
}
