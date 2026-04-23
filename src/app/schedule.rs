//! 定时任务调度：间隔模式 / 指定时间点模式。

use anyhow::Result;
use chrono::{Datelike, Local};
use log::{error, info};
use stock_analysis::pipeline::{AnalysisPipeline, PipelineConfig};

use crate::app::get_max_workers;
use crate::cli::Args;

/// 运行定时任务：按命令行/环境变量选择间隔模式或指定时间点模式。
pub fn run_scheduled_analysis(stock_codes: &[String], args: &Args) -> Result<()> {
    info!("模式: 定时任务");

    let market_review_enabled = args.market_review
        || std::env::var("MARKET_REVIEW_ENABLED")
            .unwrap_or_else(|_| "false".to_string())
            .to_lowercase()
            == "true";

    info!(
        "定时任务类型: {}",
        if market_review_enabled { "大盘复盘" } else { "个股分析" }
    );

    let config = PipelineConfig {
        max_workers: get_max_workers(args),
        dry_run: args.dry_run,
        send_notification: !args.no_notify,
        single_notify: args.single_notify,
    };

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        if let Some(interval_minutes) = args.interval {
            run_interval_schedule(
                stock_codes,
                &config,
                interval_minutes,
                args.run_now,
                market_review_enabled,
            )
            .await
        } else if let Some(ref schedule_time) = args.schedule_time {
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

/// 间隔执行模式：每 N 分钟执行一次。
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

/// 指定时间点执行模式：支持多时间点与星期过滤。
async fn run_time_schedule(
    stock_codes: &[String],
    config: &PipelineConfig,
    schedule_time: &str,
    weekdays: Option<&[u32]>,
    run_now: bool,
    market_review_enabled: bool,
) -> Result<()> {
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
        return Err(anyhow::anyhow!("无效的定时时间格式，应为 HH:MM 或 HH:MM,HH:MM"));
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
        let mut next_run = None;
        let mut min_wait = chrono::TimeDelta::MAX;

        for &(target_hour, target_minute) in &time_points {
            let mut candidate = now
                .date_naive()
                .and_hms_opt(target_hour, target_minute, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap();

            // 刚执行完的时间点避免立即再次命中
            if candidate <= now + chrono::Duration::minutes(2) {
                candidate += chrono::Duration::days(1);
            }

            if let Some(days) = weekdays {
                while !days.contains(&(candidate.weekday().num_days_from_monday() + 1)) {
                    candidate += chrono::Duration::days(1);
                }
            }

            let wait = candidate - now;
            if wait > chrono::Duration::zero() && wait < min_wait {
                min_wait = wait;
                next_run = Some(candidate);
            }
        }

        let Some(next_time) = next_run else {
            return Err(anyhow::anyhow!("无法计算下次执行时间"));
        };

        let wait_duration = min_wait.to_std()?;
        let hours = wait_duration.as_secs_f64() / 3600.0;

        info!("\n下次执行时间: {}", next_time.format("%Y-%m-%d %H:%M:%S (%A)"));
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
    }
}

async fn execute_analysis_with_mode(
    stock_codes: &[String],
    config: &PipelineConfig,
    market_review_only: bool,
) {
    if market_review_only {
        match tokio::task::spawn_blocking(crate::app::modes::run_market_review_only).await {
            Ok(Ok(())) => info!("大盘复盘完成"),
            Ok(Err(e)) => error!("大盘复盘失败: {}", e),
            Err(e) => error!("大盘复盘任务执行失败: {}", e),
        }
    } else {
        execute_analysis(stock_codes, config).await;
    }
}

async fn execute_analysis(stock_codes: &[String], config: &PipelineConfig) {
    match AnalysisPipeline::new(config.clone()) {
        Ok(pipeline) => match pipeline.run(stock_codes, None).await {
            Ok(results) => {
                info!("分析完成，成功 {} 只股票", results.len());
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
            Err(e) => error!("分析失败: {}", e),
        },
        Err(e) => error!("创建分析管道失败: {}", e),
    }
}
