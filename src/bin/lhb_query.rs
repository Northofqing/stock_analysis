//! BR-162 evidence-preserving dragon-tiger query tool.
//!
//! Usage:
//! - `cargo run --bin lhb_query -- today`
//! - `cargo run --bin lhb_query -- date 20260724`

use anyhow::Result;
use clap::{Parser, Subcommand};
use stock_analysis::data_gateway::{
    DragonTigerGateway, DragonTigerSourceDisclosure, DragonTigerStockReview, GatewayBatch,
};
use stock_analysis::lhb_analyzer::parse_dragon_tiger_date;

const TOP_N: usize = 10;

#[derive(Debug, Parser)]
#[command(name = "lhb_query")]
#[command(about = "统一 Gateway 龙虎榜真实批次查询工具", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// 查询最近已完成交易日的龙虎榜
    Today,
    /// 查询指定交易日的龙虎榜
    Date {
        /// 日期格式: YYYYMMDD 或 YYYY-MM-DD
        date: String,
    },
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    env_logger::init();

    let trading_date = match Cli::parse().command {
        Commands::Today => stock_analysis::calendar::latest_completed_trading_day_at(
            chrono::Local::now().naive_local(),
        ),
        Commands::Date { date } => parse_dragon_tiger_date(&date)?,
    };
    let batch = DragonTigerGateway::new()
        .market_review(trading_date, TOP_N as u32, TOP_N)
        .await?;
    render_batch(trading_date, batch);
    Ok(())
}

fn render_batch(trading_date: chrono::NaiveDate, batch: GatewayBatch<DragonTigerStockReview>) {
    match batch {
        GatewayBatch::VerifiedEmpty(evidence) => {
            println!(
                "{} 龙虎榜为来源确认空批次 | provider={:?} source={} source_at={} observed_at={} batch_id={}",
                trading_date,
                evidence.provider,
                evidence.source,
                evidence.source_at.as_deref().unwrap_or("缺失"),
                evidence.observed_at,
                evidence.batch_id
            );
        }
        GatewayBatch::Available { records, evidence } => {
            println!(
                "{} 龙虎榜真实批次 | provider={:?} source={} source_at={} observed_at={} batch_id={} stocks={}",
                trading_date,
                evidence.provider,
                evidence.source,
                evidence.source_at.as_deref().unwrap_or("缺失"),
                evidence.observed_at,
                evidence.batch_id,
                records.len()
            );
            for (index, stock) in records.iter().enumerate() {
                println!(
                    "\n{}. {:?} {} | 排名净买入 {:.2} 万 | 源披露 {} 条",
                    index + 1,
                    stock.exchange,
                    stock.code,
                    stock.ranking_net_amount_yuan / 10_000.0,
                    stock.disclosures.len()
                );
                for disclosure in &stock.disclosures {
                    render_disclosure(disclosure);
                }
            }
        }
    }
}

fn render_disclosure(disclosure: &DragonTigerSourceDisclosure) {
    println!(
        "   TRADE_ID={} | 原因={} | 买={} | 卖={} | 净={} | 换手率={}",
        disclosure.trade_id,
        disclosure.reason.as_deref().unwrap_or("缺失"),
        format_optional_amount(disclosure.buy_amount_yuan),
        format_optional_amount(disclosure.sell_amount_yuan),
        format_optional_amount(disclosure.net_amount_yuan),
        disclosure
            .turnover_rate_pct
            .map(|value| format!("{value:.4}%"))
            .unwrap_or_else(|| "缺失".to_string())
    );
    for seat in &disclosure.seats {
        println!(
            "      {:?}{} {} | 成交={:.2}万 | 买={} | 卖={} | 净={}",
            seat.side,
            seat.rank,
            seat.seat_name,
            seat.amount_yuan / 10_000.0,
            format_optional_amount(seat.buy_amount_yuan),
            format_optional_amount(seat.sell_amount_yuan),
            format_optional_amount(seat.net_amount_yuan)
        );
    }
}

fn format_optional_amount(value: Option<f64>) -> String {
    value
        .map(|value| format!("{:.2}万", value / 10_000.0))
        .unwrap_or_else(|| "缺失".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn br162_cli_exposes_only_today_and_date_queries() {
        assert!(matches!(
            Cli::try_parse_from(["lhb_query", "today"])
                .expect("today command")
                .command,
            Commands::Today
        ));
        assert!(matches!(
            Cli::try_parse_from(["lhb_query", "date", "20260724"])
                .expect("date command")
                .command,
            Commands::Date { .. }
        ));
        assert!(Cli::try_parse_from(["lhb_query", "stock", "600519"]).is_err());
        assert!(Cli::try_parse_from(["lhb_query", "screen", "60"]).is_err());
    }

    #[test]
    fn missing_optional_amount_is_rendered_as_missing_not_zero() {
        assert_eq!(format_optional_amount(None), "缺失");
        assert_eq!(format_optional_amount(Some(12_340.0)), "1.23万");
    }
}
