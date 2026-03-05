//! 龙虎榜数据查询工具
//! 
//! 用法:
//! cargo run --bin lhb_query -- today              # 查看今日龙虎榜
//! cargo run --bin lhb_query -- stock 600519       # 查看个股龙虎榜历史
//! cargo run --bin lhb_query -- screen 60          # 筛选评分>=60的股票

use stock_analysis::lhb_analyzer::LhbDataFetcher;
use clap::{Parser, Subcommand};
use anyhow::Result;

#[derive(Parser)]
#[command(name = "lhb_query")]
#[command(about = "龙虎榜数据查询工具", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 查看今日龙虎榜
    Today,
    /// 查看指定日期龙虎榜
    Date {
        /// 日期，格式: 20260128
        date: String,
    },
    /// 查看个股龙虎榜历史
    Stock {
        /// 股票代码
        code: String,
        /// 查询天数，默认30天
        #[arg(short, long, default_value = "30")]
        days: i32,
    },
    /// 筛选优质龙虎榜股票
    Screen {
        /// 最低评分，默认60
        #[arg(short, long, default_value = "60")]
        min_score: i32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    
    let cli = Cli::parse();
    let fetcher = LhbDataFetcher::new()?;

    match cli.command {
        Commands::Today => {
            println!("📊 正在获取今日龙虎榜数据...\n");
            let records = fetcher.get_today_lhb().await?;
            
            if records.is_empty() {
                println!("今日暂无龙虎榜数据");
                return Ok(());
            }

            println!("📈 今日龙虎榜 ({} 只股票)\n", records.len());
            println!("{:<10} {:<12} {:<8} {:>12} {:>12} {:>12}", 
                "代码", "名称", "涨跌幅%", "净买入(万)", "买入(万)", "卖出(万)");
            println!("{}", "-".repeat(80));

            for record in records.iter().take(50) {
                println!("{:<10} {:<12} {:>7.2}% {:>12.0} {:>12.0} {:>12.0}",
                    record.code,
                    record.name,
                    record.pct_change,
                    record.net_amount,
                    record.buy_amount,
                    record.sell_amount,
                );
            }
        }

        Commands::Date { date } => {
            println!("📊 正在获取 {} 的龙虎榜数据...\n", date);
            let records = fetcher.get_lhb_by_date(&date).await?;
            
            if records.is_empty() {
                println!("{} 暂无龙虎榜数据", date);
                return Ok(());
            }

            println!("📈 {} 龙虎榜 ({} 只股票)\n", date, records.len());
            println!("{:<10} {:<12} {:<8} {:>12} {:>12} {:<30}", 
                "代码", "名称", "涨跌幅%", "净买入(万)", "成交占比%", "上榜原因");
            println!("{}", "-".repeat(100));

            for record in records {
                println!("{:<10} {:<12} {:>7.2}% {:>12.0} {:>11.2}% {:<30}",
                    record.code,
                    record.name,
                    record.pct_change,
                    record.net_amount,
                    record.lhb_ratio,
                    record.reason.chars().take(28).collect::<String>(),
                );
            }
        }

        Commands::Stock { code, days } => {
            println!("📊 正在分析 {} 的龙虎榜数据...\n", code);
            
            // 获取历史记录
            let records = fetcher.get_stock_lhb_history(&code, days).await?;
            
            if records.is_empty() {
                println!("{} 最近{}天未上榜龙虎榜", code, days);
                return Ok(());
            }

            // 获取分析结果
            let analysis = fetcher.analyze_stock_lhb(&code).await?;

            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("📈 {} {} - 龙虎榜分析报告", analysis.code, analysis.name);
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

            println!("📊 综合指标:");
            println!("   最近{}天上榜次数: {} 次", days, analysis.recent_count);
            println!("   机构参与度评分: {} 分", analysis.inst_score);
            println!("   游资活跃度评分: {} 分", analysis.hot_money_score);
            println!("   综合评分: {} 分", analysis.total_score);
            
            // 评级
            let rating = if analysis.total_score >= 80 {
                "⭐⭐⭐⭐⭐ 强烈推荐"
            } else if analysis.total_score >= 60 {
                "⭐⭐⭐⭐ 推荐"
            } else if analysis.total_score >= 40 {
                "⭐⭐⭐ 中性"
            } else {
                "⭐⭐ 观望"
            };
            println!("   评级: {}\n", rating);

            if !analysis.reason.is_empty() {
                println!("💡 推荐理由:");
                println!("   {}\n", analysis.reason);
            }

            if !analysis.risk_warning.is_empty() {
                println!("⚠️  风险提示:");
                println!("   {}\n", analysis.risk_warning);
            }

            println!("📋 上榜明细:\n");
            println!("{:<12} {:<8} {:>12} {:>12} {:>12} {:<30}", 
                "日期", "涨跌幅%", "净买入(万)", "买入(万)", "卖出(万)", "上榜原因");
            println!("{}", "-".repeat(100));

            for record in records {
                println!("{:<12} {:>7.2}% {:>12.0} {:>12.0} {:>12.0} {:<30}",
                    record.trade_date,
                    record.pct_change,
                    record.net_amount,
                    record.buy_amount,
                    record.sell_amount,
                    record.reason.chars().take(28).collect::<String>(),
                );
            }
        }

        Commands::Screen { min_score } => {
            println!("🔍 正在筛选龙虎榜优质股票（评分>={})...\n", min_score);
            
            let results = fetcher.screen_lhb_stocks(min_score).await?;
            
            if results.is_empty() {
                println!("未找到符合条件的股票");
                return Ok(());
            }

            println!("✅ 找到 {} 只优质股票\n", results.len());
            println!("{:<10} {:<12} {:>8} {:>8} {:>8} {:<40}", 
                "代码", "名称", "综合分", "机构分", "游资分", "推荐理由");
            println!("{}", "-".repeat(110));

            for analysis in results {
                println!("{:<10} {:<12} {:>8} {:>8} {:>8} {:<40}",
                    analysis.code,
                    analysis.name,
                    analysis.total_score,
                    analysis.inst_score,
                    analysis.hot_money_score,
                    analysis.reason.chars().take(38).collect::<String>(),
                );
            }
        }
    }

    Ok(())
}
