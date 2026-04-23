//! 命令行参数定义
//!
//! 从 main.rs 拆分出来，减少主程序噪音。

use clap::{ArgAction, Parser};

#[derive(Parser, Debug)]
#[command(
    name = "stock_analysis",
    about = "A股自选股智能分析系统",
    long_about = "完整的股票分析系统，包含数据获取、趋势分析、AI分析和多渠道通知"
)]
pub struct Args {
    /// 启用调试模式
    #[arg(long, action = ArgAction::SetTrue)]
    pub debug: bool,

    /// 仅获取数据，不进行分析
    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,

    /// 指定要分析的股票代码（逗号分隔）
    #[arg(long, value_delimiter = ',')]
    pub stocks: Option<Vec<String>>,

    /// 不发送推送通知
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_notify: bool,

    /// 单股推送模式（每分析完一只立即推送）
    #[arg(long, action = ArgAction::SetTrue)]
    pub single_notify: bool,

    /// 并发线程数
    #[arg(long)]
    pub workers: Option<usize>,

    /// 启用定时任务模式
    #[arg(long, action = ArgAction::SetTrue)]
    pub schedule: bool,

    /// 定时时间 (格式: HH:MM 或 "HH:MM,HH:MM" 多个时间点)
    #[arg(long)]
    pub schedule_time: Option<String>,

    /// 定时间隔 (分钟)，例如每30分钟执行一次
    #[arg(long)]
    pub interval: Option<u64>,

    /// 指定执行星期 (逗号分隔，1=周一, 7=周日)
    #[arg(long, value_delimiter = ',')]
    pub weekdays: Option<Vec<u32>>,

    /// 立即执行一次，不等待下次定时
    #[arg(long, action = ArgAction::SetTrue)]
    pub run_now: bool,

    /// 仅运行大盘复盘
    #[arg(long, action = ArgAction::SetTrue)]
    pub market_review: bool,

    /// 跳过大盘复盘
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_market_review: bool,

    /// 龙虎榜选股模式（分析今日龙虎榜上榜股票）
    #[arg(long, action = ArgAction::SetTrue)]
    pub lhb_mode: bool,

    /// 龙虎榜日期（格式：YYYYMMDD，默认为今日）
    #[arg(long)]
    pub lhb_date: Option<String>,

    /// 龙虎榜最低评分（默认60分）
    #[arg(long, default_value = "60")]
    pub lhb_min_score: i32,
}
