//! 应用层逻辑：从 main.rs 拆分出来的启动校验、股票列表装配、运行模式、定时调度。
//!
//! 划分：
//! - [`bootstrap`]：启动校验 + 自选股装配 + 宏观 AI 推荐
//! - [`modes`]：三种运行模式（单次 / 大盘复盘 / 龙虎榜选股）
//! - [`schedule`]：定时任务调度

pub mod bootstrap;
pub mod modes;
pub mod schedule;

pub use bootstrap::{build_stock_list, validate_startup_config};
pub use modes::{run_analysis, run_lhb_analysis, run_market_review_only};
pub use schedule::run_scheduled_analysis;

use crate::cli::Args;

/// 获取最大并发数：优先级 命令行参数 > 环境变量 > 默认值 3
pub(crate) fn get_max_workers(args: &Args) -> usize {
    args.workers
        .or_else(|| {
            std::env::var("MAX_WORKERS")
                .ok()
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(3)
}
