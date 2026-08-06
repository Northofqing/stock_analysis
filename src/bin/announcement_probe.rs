//! 公告批次排序探针: 验证 cninfo 返回顺序 (最新优先 vs 正序/其他)。
//! 判定依据: 批次内首/中/尾样本的 published_at 分布 + 是否覆盖全天时段。
//!
//! 用法:
//!   cargo run --bin announcement_probe -- <date> [limit]
//!   例: cargo run --bin announcement_probe -- 2026-08-06 300

use anyhow::{bail, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let date_str = args.get(1).map(String::as_str).unwrap_or("2026-08-06");
    let limit: u32 = args
        .get(2)
        .map(|s| s.parse().unwrap_or(100))
        .unwrap_or(100);
    let date = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")?;
    // BR-159 审计依赖数据库; 独立 bin 需先初始化 (用主库, 审计表只追加不污染)
    stock_analysis::database::DatabaseManager::init(Some(
        std::path::PathBuf::from("data/stock_analysis.db"),
    ))
    .map_err(|error| anyhow::anyhow!("数据库初始化失败: {error}"))?;

    let batch = stock_analysis::data_gateway::EventCalendarGateway::new()
        .market_announcements(date, limit)
        .await?;
    let records: Vec<&stock_analysis::data_gateway::EventAnnouncement> = match &batch {
        stock_analysis::data_gateway::GatewayBatch::Available { records, evidence } => {
            println!(
                "批次: provider={:?} batch_id={} records={}",
                evidence.provider,
                evidence.batch_id,
                records.len()
            );
            records.iter().collect()
        }
        other => bail!("不可用: {other:?}"),
    };
    if records.is_empty() {
        bail!("空批次");
    }

    // 排序判定: published_at 是否单调 (倒序=最新优先, 正序=最早优先)
    let times: Vec<&str> = records.iter().map(|r| r.published_at.as_str()).collect();
    let mut desc_ok = true;
    let mut asc_ok = true;
    for pair in times.windows(2) {
        if pair[0] < pair[1] {
            desc_ok = false;
        }
        if pair[0] > pair[1] {
            asc_ok = false;
        }
    }
    println!(
        "排序: 倒序(最新优先)={desc_ok} 正序(最早优先)={asc_ok}"
    );
    println!(
        "时间范围: {}  →  {}",
        times.first().unwrap_or(&""),
        times.last().unwrap_or(&"")
    );

    let sample = |range: std::ops::Range<usize>, label: &str| {
        println!("--- {label} ---");
        for r in records.iter().take(range.end).skip(range.start) {
            println!("  {} {} {}", r.published_at, r.code, truncate(&r.title, 24));
        }
    };
    sample(0..5, "首 5 条");
    let mid = records.len() / 2;
    sample(mid.saturating_sub(2)..mid + 3, "中间 5 条");
    sample(records.len().saturating_sub(5)..records.len(), "尾 5 条");
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
