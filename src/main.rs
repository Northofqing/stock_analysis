//! A股自选股智能分析系统 - 主程序入口
//!
//! 职责：
//! 1. 加载环境变量与解析 CLI 参数
//! 2. 初始化日志 / 启动配置校验 / 数据库
//! 3. 装配待分析股票列表
//! 4. 根据参数/环境变量分派到对应运行模式
//!
//! 具体模式逻辑拆分在 [`app`] 模块下：
//! - [`app::bootstrap`]：启动校验 + 股票列表装配
//! - [`app::modes`]：三种运行模式（单次 / 大盘复盘 / 龙虎榜选股）
//! - [`app::schedule`]：定时任务调度

mod app;
mod cli;

use anyhow::Result;
use chrono::Local;
use clap::Parser;
use env_logger::Env;
use log::{error, info};
use std::io::Write;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let args = cli::Args::parse();

    // 日志初始化（时间戳使用本地时区）
    let default_level = if args.debug { "debug" } else { "info" };
    if std::env::var("RUST_LOG").unwrap_or_default().is_empty() {
        std::env::set_var("RUST_LOG", default_level);
    }
    env_logger::Builder::from_env(Env::default().default_filter_or(default_level))
        .format(|buf, record| {
            writeln!(
                buf,
                "[{} {} {}] {}",
                Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                record.level(),
                record.target(),
                record.args()
            )
        })
        .init();

    info!("============================================================");
    info!("A股自选股智能分析系统 启动");
    info!("运行时间: {}", Local::now().format("%Y-%m-%d %H:%M:%S"));
    info!("============================================================");

    // 启动前配置校验：不合法直接 exit(1)
    app::validate_startup_config();

    // 初始化数据库
    {
        use std::path::PathBuf;
        use stock_analysis::database::DatabaseManager;
        let db_path = std::env::var("DATABASE_PATH")
            .unwrap_or_else(|_| "./data/stock_analysis.db".to_string());
        match DatabaseManager::init(Some(PathBuf::from(&db_path))) {
            Ok(_) => info!("数据库初始化完成: {}", db_path),
            Err(e) => error!("数据库初始化失败: {}（数据将不会入库）", e),
        }
    }

    // 模式分派
    let env_true = |k: &str| std::env::var(k).unwrap_or_default().to_lowercase() == "true";

    // 定时模式优先：放在最前面，启动时不预先装配股票列表。
    // 改为每次定时执行时重新读取配置（.env）并重新装配，
    // 使运行过程中对 .env / 股票池的修改即时生效。
    if args.schedule || env_true("SCHEDULE_ENABLED") {
        app::run_scheduled_analysis(&args).await?;
        info!("程序执行完成");
        return Ok(());
    }

    if args.chain_analysis {
        app::run_chain_analysis_mode(!args.no_notify).await?;
        info!("程序执行完成");
        return Ok(());
    }

    // 非定时模式：启动时装配一次待分析股票列表
    let (stock_codes, limit_up_codes, macro_ctx) = app::build_stock_list(&args).await?;
    info!(
        "待分析股票（共 {} 只）: {:?}",
        stock_codes.len(),
        stock_codes
    );

    if args.lhb_mode || env_true("LHB_MODE") {
        app::run_lhb_analysis(&args).await?;
    } else if args.market_review || env_true("MARKET_REVIEW_ENABLED") {
        info!("模式: 仅大盘复盘");
        app::run_market_review_only().await?;
    } else {
        app::run_analysis(&stock_codes, &args, &macro_ctx, limit_up_codes).await?;
    }

    info!("程序执行完成");
    Ok(())
}
