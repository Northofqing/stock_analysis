//! A股自选股智能分析系统 - 主程序
//!
//! 职责：
//! 1. 协调各模块完成股票分析流程
//! 2. 实现并发调度
//! 3. 全局异常处理
//! 4. 提供命令行入口

use anyhow::Result;
use chrono::{Datelike, Local};
use clap::{ArgAction, Parser};
use env_logger::Env;
use log::{error, info};
use stock_analysis::lhb_analyzer::LhbDataFetcher;
use stock_analysis::pipeline::{AnalysisPipeline, PipelineConfig};

#[derive(Parser, Debug)]
#[command(
    name = "stock_analysis",
    about = "A股自选股智能分析系统",
    long_about = "完整的股票分析系统，包含数据获取、趋势分析、AI分析和多渠道通知"
)]
struct Args {
    /// 启用调试模式
    #[arg(long, action = ArgAction::SetTrue)]
    debug: bool,

    /// 仅获取数据，不进行分析
    #[arg(long, action = ArgAction::SetTrue)]
    dry_run: bool,

    /// 指定要分析的股票代码（逗号分隔）
    #[arg(long, value_delimiter = ',')]
    stocks: Option<Vec<String>>,

    /// 不发送推送通知
    #[arg(long, action = ArgAction::SetTrue)]
    no_notify: bool,

    /// 单股推送模式（每分析完一只立即推送）
    #[arg(long, action = ArgAction::SetTrue)]
    single_notify: bool,

    /// 并发线程数
    #[arg(long)]
    workers: Option<usize>,

    /// 启用定时任务模式
    #[arg(long, action = ArgAction::SetTrue)]
    schedule: bool,

    /// 定时时间 (格式: HH:MM 或 "HH:MM,HH:MM" 多个时间点)
    #[arg(long)]
    schedule_time: Option<String>,

    /// 定时间隔 (分钟)，例如每30分钟执行一次
    #[arg(long)]
    interval: Option<u64>,

    /// 指定执行星期 (逗号分隔，1=周一, 7=周日)
    #[arg(long, value_delimiter = ',')]
    weekdays: Option<Vec<u32>>,

    /// 立即执行一次，不等待下次定时
    #[arg(long, action = ArgAction::SetTrue)]
    run_now: bool,

    /// 仅运行大盘复盘
    #[arg(long, action = ArgAction::SetTrue)]
    market_review: bool,

    /// 跳过大盘复盘
    #[arg(long, action = ArgAction::SetTrue)]
    no_market_review: bool,

    /// 龙虎榜选股模式（分析今日龙虎榜上榜股票）
    #[arg(long, action = ArgAction::SetTrue)]
    lhb_mode: bool,

    /// 龙虎榜日期（格式：YYYYMMDD，默认为今日）
    #[arg(long)]
    lhb_date: Option<String>,

    /// 龙虎榜最低评分（默认60分）
    #[arg(long, default_value = "60")]
    lhb_min_score: i32,
}

/// 主入口函数
fn main() -> Result<()> {
    // 加载环境变量
    dotenv::dotenv().ok();

    // 解析命令行参数
    let args = Args::parse();

    // 配置日志（如果 RUST_LOG 为空则忽略，使用默认级别）
    let default_level = if args.debug { "debug" } else { "info" };
    if std::env::var("RUST_LOG").unwrap_or_default().is_empty() {
        std::env::set_var("RUST_LOG", default_level);
    }
    env_logger::Builder::from_env(Env::default().default_filter_or(default_level)).init();

    info!("============================================================");
    info!("A股自选股智能分析系统 启动");
    info!("运行时间: {}", Local::now().format("%Y-%m-%d %H:%M:%S"));
    info!("============================================================");

    // 初始化数据库（如果配置了数据库路径）
    use std::path::PathBuf;
    use stock_analysis::database::DatabaseManager;
    let db_path =
        std::env::var("DATABASE_PATH").unwrap_or_else(|_| "./data/stock_analysis.db".to_string());
    match DatabaseManager::init(Some(PathBuf::from(&db_path))) {
        Ok(_) => info!("数据库初始化完成: {}", db_path),
        Err(e) => error!("数据库初始化失败: {}（数据将不会入库）", e),
    }

    // 读取股票列表
    let mut stock_codes: Vec<String> = if let Some(ref stocks) = args.stocks {
        stocks.clone()
    } else {
        // 从环境变量读取并去重
        std::env::var("STOCK_LIST")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect()
    };

    // 通过宏观新闻 AI 分析，追加推荐股票到自选股列表
    let macro_news_context;
    {
        let runtime = tokio::runtime::Runtime::new()?;
        let (extra_codes, macro_text) = runtime.block_on(fetch_macro_recommended_codes());
        macro_news_context = macro_text;
        if !extra_codes.is_empty() {
            let before = stock_codes.len();
            for code in &extra_codes {
                if !stock_codes.contains(code) {
                    stock_codes.push(code.clone());
                }
            }
            let added = stock_codes.len() - before;
            info!(
                "📈 宏观AI推荐 {} 只，新增追加 {} 只（去重后）",
                extra_codes.len(),
                added
            );
        }
    }

    // 追加当日龙虎榜净买入前10的股票（过滤北交所股票，代码以92开头）
    let runtime = tokio::runtime::Runtime::new()?;
    match runtime.block_on(async {
        let fetcher = LhbDataFetcher::new()?;
        fetcher.get_today_lhb().await
    }) {
        Ok(mut lhb_records) => {
            if !lhb_records.is_empty() {
                // 按净买入额降序排列，取前10
                lhb_records.sort_by(|a, b| {
                    b.net_amount
                        .partial_cmp(&a.net_amount)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                let top_n = 10;
                let before = stock_codes.len();
                for record in lhb_records.iter().take(top_n) {
                    // 过滤北交所（92开头）
                    if record.code.starts_with("92") {
                        continue;
                    }
                    if !stock_codes.contains(&record.code) {
                        info!(
                            "🐉 龙虎榜追加: {}({}) 净买入{:.0}万",
                            record.name,
                            record.code,
                            record.net_amount / 10000.0
                        );
                        stock_codes.push(record.code.clone());
                    }
                }
                let added = stock_codes.len() - before;
                info!("🐉 龙虎榜Top{} 新增追加 {} 只（去重后）", top_n, added);
            } else {
                info!("📋 今日暂无龙虎榜数据");
            }
        }
        Err(e) => {
            info!("⚠️ 获取龙虎榜数据失败（不影响正常分析）: {}", e);
        }
    }

    // 追加当日涨停股票进行分析
    let mut limit_up_code_set = std::collections::HashSet::new();
    {
        use stock_analysis::market_analyzer::MarketAnalyzer;
        match MarketAnalyzer::new(None) {
            Ok(analyzer) => {
                match analyzer.get_limit_up_stocks() {
                    Ok(limit_up_stocks) => {
                        if !limit_up_stocks.is_empty() {
                            let before = stock_codes.len();
                            for stock in &limit_up_stocks {
                                // 北交所和ST已在get_limit_up_stocks中过滤
                                limit_up_code_set.insert(stock.code.clone());
                                if !stock_codes.contains(&stock.code) {
                                    info!(
                                        "🔥 涨停追加: {}({}) 涨幅{:.2}%",
                                        stock.name, stock.code, stock.change_pct
                                    );
                                    stock_codes.push(stock.code.clone());
                                }
                            }
                            let added = stock_codes.len() - before;
                            info!(
                                "🔥 当日涨停 {} 只，新增追加 {} 只（去重后）",
                                limit_up_stocks.len(),
                                added
                            );
                        } else {
                            info!("📋 今日暂无涨停股票");
                        }
                    }
                    Err(e) => {
                        info!("⚠️ 获取涨停股票失败（不影响正常分析）: {}", e);
                    }
                }
            }
            Err(e) => {
                info!("⚠️ 创建市场分析器失败: {}", e);
            }
        }
    }

    // 追加数据库中持仓中的股票，确保持续跟踪收益
    {
        use stock_analysis::database::DatabaseManager;
        if let Ok(db) = std::panic::catch_unwind(|| DatabaseManager::get()) {
            match db.get_all_open_positions() {
                Ok(positions) => {
                    if !positions.is_empty() {
                        let before = stock_codes.len();
                        for pos in &positions {
                            if !stock_codes.contains(&pos.code) {
                                info!(
                                    "💰 持仓追加: {}({}) 买入价{:.2}",
                                    pos.name, pos.code, pos.buy_price
                                );
                                stock_codes.push(pos.code.clone());
                            }
                        }
                        let added = stock_codes.len() - before;
                        info!("💰 持仓中 {} 只，新增追加 {} 只（去重后）", positions.len(), added);
                    }
                }
                Err(e) => {
                    info!("⚠️ 查询持仓数据失败（不影响正常分析）: {}", e);
                }
            }
        }
    }

    if stock_codes.is_empty() {
        info!("⚠️ 未配置自选股列表且宏观AI未推荐股票，将仅执行大盘复盘");
    }

    info!(
        "待分析股票（共 {} 只）: {:?}",
        stock_codes.len(),
        stock_codes
    );

    // 读取环境变量配置
    let schedule_enabled = std::env::var("SCHEDULE_ENABLED")
        .unwrap_or_else(|_| "false".to_string())
        .to_lowercase()
        == "true";
    let market_review_enabled = std::env::var("MARKET_REVIEW_ENABLED")
        .unwrap_or_else(|_| "false".to_string())
        .to_lowercase()
        == "true";
    let lhb_mode_enabled = std::env::var("LHB_MODE")
        .unwrap_or_else(|_| "false".to_string())
        .to_lowercase()
        == "true";

    // 模式1: 定时任务（优先判断，因为定时任务可以包含大盘复盘）
    if args.schedule || schedule_enabled {
        info!(
            "定时任务模式已启用（来源: {}）",
            if args.schedule {
                "命令行参数"
            } else {
                "环境变量"
            }
        );
        run_scheduled_analysis(&stock_codes, &args)?;
        return Ok(());
    }

    // 模式2: 龙虎榜选股模式（单次执行）
    if args.lhb_mode || lhb_mode_enabled {
        info!(
            "模式: 龙虎榜选股分析（来源: {}）",
            if args.lhb_mode {
                "命令行参数"
            } else {
                "环境变量"
            }
        );
        run_lhb_analysis(&args)?;
        return Ok(());
    }

    // 模式3: 仅大盘复盘（单次执行）
    if args.market_review || market_review_enabled {
        info!("模式: 仅大盘复盘");
        run_market_review_only()?;
        return Ok(());
    }

    // 模式4: 正常分析流程（单次执行）
    run_analysis(&stock_codes, &args, &macro_news_context, limit_up_code_set)?;

    info!("程序执行完成");
    Ok(())
}

/// 获取最大并发数：优先级 命令行参数 > 环境变量 > 默认值3
fn get_max_workers(args: &Args) -> usize {
    args.workers
        .or_else(|| {
            std::env::var("MAX_WORKERS")
                .ok()
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(3)
}

/// 运行分析流程
fn run_analysis(
    stock_codes: &[String],
    args: &Args,
    macro_context: &str,
    limit_up_codes: std::collections::HashSet<String>,
) -> Result<()> {
    info!("模式: 单次分析");

    let config = PipelineConfig {
        max_workers: get_max_workers(args),
        dry_run: args.dry_run,
        send_notification: !args.no_notify,
        single_notify: args.single_notify,
    };

    let pipeline = AnalysisPipeline::new(config)?.with_limit_up_codes(limit_up_codes);

    // 使用tokio运行异步任务，传入已获取的宏观新闻避免重复搜索
    let runtime = tokio::runtime::Runtime::new()?;
    let mc = if macro_context.is_empty() {
        None
    } else {
        Some(macro_context.to_string())
    };
    let results = runtime.block_on(pipeline.run(stock_codes, mc))?;

    // 输出摘要（按评分从高到低排序）
    if !results.is_empty() {
        info!("\n===== 分析结果摘要 =====");
        let mut sorted_results = results.clone();
        sorted_results.sort_by(|a, b| b.sentiment_score.cmp(&a.sentiment_score));
        for r in sorted_results.iter() {
            info!(
                "{} {}({}) - {} (评分: {})",
                r.get_emoji(),
                r.name,
                r.code,
                r.operation_advice,
                r.sentiment_score
            );
        }
    }

    Ok(())
}

/// 运行定时任务
fn run_scheduled_analysis(stock_codes: &[String], args: &Args) -> Result<()> {
    info!("模式: 定时任务");

    // 检查是否是大盘复盘模式
    let market_review_enabled = args.market_review
        || std::env::var("MARKET_REVIEW_ENABLED")
            .unwrap_or_else(|_| "false".to_string())
            .to_lowercase()
            == "true";

    if market_review_enabled {
        info!("定时任务类型: 大盘复盘");
    } else {
        info!("定时任务类型: 个股分析");
    }

    let config = PipelineConfig {
        max_workers: get_max_workers(args),
        dry_run: args.dry_run,
        send_notification: !args.no_notify,
        single_notify: args.single_notify,
    };

    // 创建tokio运行时
    let runtime = tokio::runtime::Runtime::new()?;

    runtime.block_on(async {
        // 判断定时模式
        if let Some(interval_minutes) = args.interval {
            // 模式1: 间隔执行
            run_interval_schedule(
                stock_codes,
                &config,
                interval_minutes,
                args.run_now,
                market_review_enabled,
            )
            .await
        } else if let Some(ref schedule_time) = args.schedule_time {
            // 模式2: 指定时间点执行（命令行参数优先）
            run_time_schedule(
                stock_codes,
                &config,
                schedule_time,
                args.weekdays.as_deref(),
                args.run_now,
                market_review_enabled,
            )
            .await
        } else {
            // 模式3: 使用环境变量配置
            let env_time = std::env::var("SCHEDULE_TIME").unwrap_or_else(|_| "09:30".to_string());
            info!("使用环境变量定时配置: SCHEDULE_TIME={}", env_time);
            run_time_schedule(
                stock_codes,
                &config,
                &env_time,
                args.weekdays.as_deref(),
                args.run_now,
                market_review_enabled,
            )
            .await
        }
    })
}

/// 间隔执行模式
async fn run_interval_schedule(
    stock_codes: &[String],
    config: &PipelineConfig,
    interval_minutes: u64,
    run_now: bool,
    market_review_enabled: bool,
) -> Result<()> {
    info!("定时模式: 每 {} 分钟执行一次", interval_minutes);

    if run_now {
        info!("立即执行首次分析...");
        execute_analysis_with_mode(stock_codes, config, market_review_enabled).await;
    }

    let interval_duration = std::time::Duration::from_secs(interval_minutes * 60);
    info!("定时任务已启动，按 Ctrl+C 退出...");

    let mut execution_count = if run_now { 1 } else { 0 };

    loop {
        let next_time = Local::now() + chrono::Duration::seconds(interval_minutes as i64 * 60);
        info!(
            "下次执行时间: {} ({:.1}分钟后)",
            next_time.format("%Y-%m-%d %H:%M:%S"),
            interval_minutes
        );

        tokio::time::sleep(interval_duration).await;

        execution_count += 1;
        info!("开始第 {} 次定时分析...", execution_count);
        execute_analysis_with_mode(stock_codes, config, market_review_enabled).await;
    }
}

/// 指定时间点执行模式
async fn run_time_schedule(
    stock_codes: &[String],
    config: &PipelineConfig,
    schedule_time: &str,
    weekdays: Option<&[u32]>,
    run_now: bool,
    market_review_enabled: bool,
) -> Result<()> {
    // 解析多个时间点
    let time_points: Vec<(u32, u32)> = schedule_time
        .split(',')
        .filter_map(|t| {
            let parts: Vec<&str> = t.trim().split(':').collect();
            if parts.len() == 2 {
                if let (Ok(h), Ok(m)) = (parts[0].parse(), parts[1].parse()) {
                    return Some((h, m));
                }
            }
            None
        })
        .collect();

    if time_points.is_empty() {
        return Err(anyhow::anyhow!(
            "无效的定时时间格式，应为 HH:MM 或 HH:MM,HH:MM"
        ));
    }

    let weekdays_str = if let Some(days) = weekdays {
        let day_names = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
        let names: Vec<String> = days
            .iter()
            .filter_map(|&d| {
                if (1..=7).contains(&d) {
                    Some(day_names[(d - 1) as usize].to_string())
                } else {
                    None
                }
            })
            .collect();
        format!(" (仅{}执行)", names.join(","))
    } else {
        String::from(" (每日执行)")
    };

    info!(
        "定时模式: 每日 {:?} 执行{}",
        time_points
            .iter()
            .map(|(h, m)| format!("{}:{:02}", h, m))
            .collect::<Vec<_>>(),
        weekdays_str
    );

    if run_now {
        info!("立即执行首次分析...");
        execute_analysis_with_mode(stock_codes, config, market_review_enabled).await;
    }

    info!("\n定时任务已启动，按 Ctrl+C 退出...\n");

    loop {
        let now = Local::now();

        // 找到下一个执行时间
        let mut next_run = None;
        let mut min_wait = chrono::Duration::max_value(); // TimeDelta::MAX in newer versions

        for &(target_hour, target_minute) in &time_points {
            let mut candidate = now
                .date_naive()
                .and_hms_opt(target_hour, target_minute, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap();

            // 如果今天的时间已过或非常接近（2分钟内），跳到明天
            // 这样可以避免刚执行完的时间点被立即再次选中
            if candidate <= now + chrono::Duration::minutes(2) {
                candidate += chrono::Duration::days(1);
            }

            // 如果指定了星期，找到下一个符合的日期
            if let Some(days) = weekdays {
                while !days.contains(&(candidate.weekday().num_days_from_monday() + 1)) {
                    candidate += chrono::Duration::days(1);
                }
            }

            let wait = candidate - now;
            // 确保等待时间为正值
            if wait > chrono::Duration::zero() && wait < min_wait {
                min_wait = wait;
                next_run = Some(candidate);
            }
        }

        if let Some(next_time) = next_run {
            let wait_duration = min_wait.to_std()?;
            let hours = wait_duration.as_secs_f64() / 3600.0;

            info!(
                "\n下次执行时间: {}",
                next_time.format("%Y-%m-%d %H:%M:%S (%A)")
            );
            if hours >= 24.0 {
                info!("等待 {:.1} 天...", hours / 24.0);
            } else if hours >= 1.0 {
                info!("等待 {:.1} 小时...", hours);
            } else {
                info!("等待 {:.0} 分钟...", wait_duration.as_secs_f64() / 60.0);
            }

            tokio::time::sleep(wait_duration).await;

            info!(
                "\n\n开始定时分析 [{}]...",
                Local::now().format("%Y-%m-%d %H:%M:%S")
            );
            execute_analysis_with_mode(stock_codes, config, market_review_enabled).await;
            info!("\n");
        } else {
            return Err(anyhow::anyhow!("无法计算下次执行时间"));
        }
    }
}

/// 执行分析（根据模式选择）
async fn execute_analysis_with_mode(
    stock_codes: &[String],
    config: &PipelineConfig,
    market_review_only: bool,
) {
    if market_review_only {
        // 大盘复盘模式 - 使用 spawn_blocking 避免在 async 上下文中调用 blocking 代码
        match tokio::task::spawn_blocking(run_market_review_only).await {
            Ok(Ok(())) => {
                info!("大盘复盘完成");
            }
            Ok(Err(e)) => {
                error!("大盘复盘失败: {}", e);
            }
            Err(e) => {
                error!("大盘复盘任务执行失败: {}", e);
            }
        }
    } else {
        // 个股分析模式
        execute_analysis(stock_codes, config).await;
    }
}

/// 执行分析（封装错误处理）
async fn execute_analysis(stock_codes: &[String], config: &PipelineConfig) {
    match AnalysisPipeline::new(config.clone()) {
        Ok(pipeline) => {
            match pipeline.run(stock_codes, None).await {
                Ok(results) => {
                    info!("分析完成，成功 {} 只股票", results.len());

                    // 输出简要结果
                    if !results.is_empty() {
                        let mut sorted = results.clone();
                        sorted.sort_by(|a, b| b.sentiment_score.cmp(&a.sentiment_score));
                        for r in sorted.iter().take(5) {
                            info!(
                                "  {} {}({}) - {} (评分: {})",
                                r.get_emoji(),
                                r.name,
                                r.code,
                                r.operation_advice,
                                r.sentiment_score
                            );
                        }
                    }
                }
                Err(e) => {
                    error!("分析失败: {}", e);
                }
            }
        }
        Err(e) => {
            error!("创建分析管道失败: {}", e);
        }
    }
}

/// 通过宏观新闻 AI 分析，返回 (推荐的 A 股代码列表, 宏观新闻全文)
/// 宏观新闻全文会传递给 pipeline 避免重复搜索
async fn fetch_macro_recommended_codes() -> (Vec<String>, String) {
    use stock_analysis::analyzer::get_analyzer;
    use stock_analysis::search_service::get_search_service;

    info!("📡 正在获取宏观新闻并由 AI 分析推荐 A 股...");
    let search_service = get_search_service();
    let mc = match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        search_service.search_macro_news(3),
    )
    .await
    {
        Ok(text) if !text.is_empty() => {
            info!("✓ 宏观新闻获取成功，共 {} 字符", text.len());
            text
        }
        Ok(_) => {
            log::warn!("宏观新闻为空，跳过AI推荐");
            return (vec![], String::new());
        }
        Err(_) => {
            log::warn!("宏观新闻获取超时(15s)，跳过AI推荐");
            return (vec![], String::new());
        }
    };

    let analyzer_clone = {
        let guard = get_analyzer().lock().unwrap();
        if guard.is_available() {
            Some(guard.clone())
        } else {
            None
        }
    };
    let Some(analyzer) = analyzer_clone else {
        log::warn!("AI 模型未配置，跳过宏观推荐");
        return (vec![], mc);
    };

    info!("🤖 正在调用 AI 分析宏观推荐（最多等待 120s）...");
    match tokio::time::timeout(
        std::time::Duration::from_secs(120),
        analyzer.analyze_macro_recommendations(&mc),
    )
    .await
    {
        Ok(Ok(rec_text)) => {
            info!("========== 宏观驱动 A 股推荐 ==========\n{}\n========================================", rec_text);
            // 保存推荐报告到本地
            let date_str = chrono::Local::now().format("%Y%m%d").to_string();
            let filename = format!("reports/macro_recommendations_{}.md", date_str);
            let header = format!(
                "# 📈 宏观驱动 A 股推荐报告\n\n**生成时间**: {}\n\n---\n\n## 今日宏观背景\n\n{}\n\n---\n\n{}\n",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                mc,
                rec_text
            );
            if let Err(e) = std::fs::write(&filename, &header) {
                log::warn!("宏观推荐报告保存失败: {}", e);
            } else {
                info!("✓ 宏观推荐报告已保存: {}", filename);
            }
            // 优先从【推荐代码】行提取（更可靠），回退到全文正则
            let re = regex::Regex::new(r"\b([036]\d{5})\b").unwrap();
            let code_line_text = rec_text
                .lines()
                .find(|line| line.contains("【推荐代码】"))
                .unwrap_or(&rec_text);
            let mut codes: Vec<String> = re
                .captures_iter(code_line_text)
                .map(|cap| cap[1].to_string())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            // 如果专用行没提取到，回退到全文提取
            if codes.is_empty() {
                codes = re
                    .captures_iter(&rec_text)
                    .map(|cap| cap[1].to_string())
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
            }
            codes.sort();
            info!(
                "✅ 从宏观推荐中提取到 {} 只股票代码: {:?}",
                codes.len(),
                codes
            );
            (codes, mc)
        }
        Ok(Err(e)) => {
            log::warn!("宏观推荐生成失败: {}", e);
            (vec![], mc)
        }
        Err(_) => {
            log::warn!("宏观推荐 AI 调用超时(120s)，跳过");
            (vec![], mc)
        }
    }
}

/// 仅运行大盘复盘
fn run_market_review_only() -> Result<()> {
    use stock_analysis::market_analyzer::MarketAnalyzer;
    use stock_analysis::notification::NotificationService;

    let analyzer = MarketAnalyzer::new(None)?;
    let overview = analyzer.get_market_overview()?;

    info!("市场概览: {:?}", overview);

    // 生成报告
    let report = analyzer.generate_template_review(&overview);

    // 保存报告
    let notifier = NotificationService::from_env();
    let date_str = Local::now().format("%Y%m%d").to_string();
    let filename = format!("market_review_{}.md", date_str);
    notifier.save_report_to_file(&report, Some(&filename))?;

    info!("大盘复盘完成");
    Ok(())
}

/// 龙虎榜选股分析模式
fn run_lhb_analysis(args: &Args) -> Result<()> {
    use stock_analysis::database::DatabaseManager;
    use stock_analysis::lhb_analyzer::{LhbAnalysis, LhbDataFetcher, LhbRecord};

    // 数据库已在 main() 中初始化，直接获取实例
    let db = DatabaseManager::get();

    // 清理过期缓存并去重
    if let Ok(deleted) = db.clean_old_lhb_data(60) {
        if deleted > 0 {
            info!("已清理 {} 条过期龙虎榜缓存", deleted);
        }
    }
    if let Ok(deleted) = db.dedupe_lhb_data() {
        if deleted > 0 {
            info!("已去重 {} 条龙虎榜缓存", deleted);
        }
    }

    info!("开始获取龙虎榜数据...");

    let runtime = tokio::runtime::Runtime::new()?;

    let (good_stocks, stock_codes) = runtime.block_on(async {
        let fetcher = LhbDataFetcher::new()?;

        // 从环境变量或命令行参数获取配置
        let lhb_date = args.lhb_date.clone().or_else(|| {
            std::env::var("LHB_DATE")
                .ok()
                .filter(|s| !s.trim().is_empty())
        });
        let lhb_min_score = if args.lhb_min_score != 60 {
            // 命令行参数被修改了
            args.lhb_min_score
        } else {
            // 使用环境变量或默认值
            std::env::var("LHB_MIN_SCORE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(60)
        };

        // 1. 获取龙虎榜数据
        let today_lhb = if let Some(date) = &lhb_date {
            info!("正在获取 {} 的龙虎榜数据...", date);
            fetcher.get_lhb_by_date(date).await?
        } else {
            let today = chrono::Local::now().format("%Y%m%d").to_string();
            info!("正在获取今日 ({}) 的龙虎榜数据...", today);
            fetcher.get_today_lhb().await?
        };

        if today_lhb.is_empty() {
            info!("今日无龙虎榜数据");
            return Ok::<(Vec<(LhbRecord, LhbAnalysis)>, Vec<String>), anyhow::Error>((
                Vec::<(LhbRecord, LhbAnalysis)>::new(),
                Vec::<String>::new(),
            ));
        }

        // 去重：同一股票只保留一条记录
        let mut seen_codes = std::collections::HashSet::new();
        let mut unique_lhb = Vec::new();
        for record in today_lhb.into_iter() {
            if seen_codes.insert(record.code.clone()) {
                unique_lhb.push(record);
            }
        }

        info!("获取到 {} 只龙虎榜股票（去重后）", unique_lhb.len());

        // 2. 逐个分析龙虎榜股票
        let mut good_stocks = Vec::new();

        for (i, record) in unique_lhb.iter().enumerate() {
            if i > 0 && i % 10 == 0 {
                info!("已处理 {}/{} 只股票", i, unique_lhb.len());
            }

            // 分析个股龙虎榜
            match fetcher.analyze_stock_lhb(&record.code).await {
                Ok(analysis) => {
                    if analysis.total_score >= lhb_min_score {
                        good_stocks.push((record.clone(), analysis));
                    }
                }
                Err(e) => {
                    log::warn!("分析 {} 失败: {}", record.code, e);
                }
            }

            // 避免请求过快
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }

        if good_stocks.is_empty() {
            info!("未找到评分≥{}的股票", lhb_min_score);
            return Ok::<(Vec<(LhbRecord, LhbAnalysis)>, Vec<String>), anyhow::Error>((
                Vec::<(LhbRecord, LhbAnalysis)>::new(),
                Vec::<String>::new(),
            ));
        }

        // 按评分排序
        good_stocks.sort_by(|a, b| b.1.total_score.cmp(&a.1.total_score));

        info!("\n筛选到 {} 只优质股票:", good_stocks.len());
        for (record, analysis) in &good_stocks {
            info!(
                "  {} {} 评分:{} (机构:{} 游资:{})",
                record.code,
                record.name,
                analysis.total_score,
                analysis.inst_score,
                analysis.hot_money_score
            );
        }

        // 3. 提取股票代码进行完整分析（过滤北交所股票，代码以92开头）
        let stock_codes: Vec<String> = good_stocks
            .iter()
            .filter(|(record, _)| !record.code.starts_with("92")) // 过滤北交所
            .map(|(record, _)| record.code.clone())
            .collect();

        if stock_codes.is_empty() {
            info!("过滤后无有效股票");
            return Ok::<(Vec<(LhbRecord, LhbAnalysis)>, Vec<String>), anyhow::Error>((
                good_stocks,
                Vec::<String>::new(),
            ));
        }

        info!("\n开始对 {} 只股票进行完整技术分析...", stock_codes.len());

        Ok((good_stocks, stock_codes))
    })?;

    if stock_codes.is_empty() {
        return Ok(());
    }

    // 4. 使用现有分析管道（在同步上下文创建，避免异步上下文drop）
    let config = PipelineConfig {
        max_workers: get_max_workers(args),
        dry_run: args.dry_run,
        send_notification: !args.no_notify,
        single_notify: args.single_notify,
    };

    let pipeline = AnalysisPipeline::new(config)?;
    let results = runtime.block_on(pipeline.run(&stock_codes, None))?;

    // 5. 生成综合报告
    info!("\n===== 龙虎榜选股分析结果 =====");

    if !results.is_empty() {
        let mut sorted_results = results.clone();
        sorted_results.sort_by(|a, b| b.sentiment_score.cmp(&a.sentiment_score));

        for r in sorted_results.iter() {
            // 找到对应的龙虎榜分析
            let lhb_info = good_stocks
                .iter()
                .find(|(record, _)| record.code == r.code)
                .map(|(_, analysis)| analysis);

            if let Some(lhb) = lhb_info {
                info!(
                    "{} {}({}) - 技术评分:{} 龙虎榜评分:{} - {}",
                    r.get_emoji(),
                    r.name,
                    r.code,
                    r.sentiment_score,
                    lhb.total_score,
                    r.operation_advice
                );
            } else {
                info!(
                    "{} {}({}) - 评分:{} - {}",
                    r.get_emoji(),
                    r.name,
                    r.code,
                    r.sentiment_score,
                    r.operation_advice
                );
            }
        }
    }

    info!("\n龙虎榜选股分析完成");

    Ok(())
}
