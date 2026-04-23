//! 三种运行模式：单次分析 / 仅大盘复盘 / 龙虎榜选股分析。

use anyhow::Result;
use chrono::Local;
use log::info;
use stock_analysis::pipeline::{AnalysisPipeline, PipelineConfig};

use crate::app::get_max_workers;
use crate::cli::Args;

/// 单次分析流程（命令行默认模式）。
pub fn run_analysis(
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

    let runtime = tokio::runtime::Runtime::new()?;
    let mc = if macro_context.is_empty() {
        None
    } else {
        Some(macro_context.to_string())
    };
    let results = runtime.block_on(pipeline.run(stock_codes, mc))?;

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

/// 仅运行大盘复盘。
pub fn run_market_review_only() -> Result<()> {
    use stock_analysis::market_analyzer::MarketAnalyzer;
    use stock_analysis::notification::NotificationService;

    let analyzer = MarketAnalyzer::new(None)?;
    let overview = analyzer.get_market_overview()?;
    info!("市场概览: {:?}", overview);

    let report = analyzer.generate_template_review(&overview);
    let notifier = NotificationService::from_env();
    let filename = format!("market_review_{}.md", Local::now().format("%Y%m%d"));
    notifier.save_report_to_file(&report, Some(&filename))?;

    info!("大盘复盘完成");
    Ok(())
}

/// 龙虎榜选股分析模式。
pub fn run_lhb_analysis(args: &Args) -> Result<()> {
    use stock_analysis::database::DatabaseManager;
    use stock_analysis::lhb_analyzer::{LhbAnalysis, LhbDataFetcher, LhbRecord};

    let db = DatabaseManager::get();
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

        let lhb_date = args.lhb_date.clone().or_else(|| {
            std::env::var("LHB_DATE").ok().filter(|s| !s.trim().is_empty())
        });
        let lhb_min_score = if args.lhb_min_score != 60 {
            args.lhb_min_score
        } else {
            std::env::var("LHB_MIN_SCORE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(60)
        };

        let today_lhb = if let Some(date) = &lhb_date {
            info!("正在获取 {} 的龙虎榜数据...", date);
            fetcher.get_lhb_by_date(date).await?
        } else {
            let today = Local::now().format("%Y%m%d").to_string();
            info!("正在获取今日 ({}) 的龙虎榜数据...", today);
            fetcher.get_today_lhb().await?
        };

        if today_lhb.is_empty() {
            info!("今日无龙虎榜数据");
            return Ok::<(Vec<(LhbRecord, LhbAnalysis)>, Vec<String>), anyhow::Error>((
                vec![],
                vec![],
            ));
        }

        // 同股票去重
        let mut seen = std::collections::HashSet::new();
        let unique_lhb: Vec<_> = today_lhb
            .into_iter()
            .filter(|r| seen.insert(r.code.clone()))
            .collect();
        info!("获取到 {} 只龙虎榜股票（去重后）", unique_lhb.len());

        let mut good_stocks = Vec::new();
        for (i, record) in unique_lhb.iter().enumerate() {
            if i > 0 && i % 10 == 0 {
                info!("已处理 {}/{} 只股票", i, unique_lhb.len());
            }
            match fetcher.analyze_stock_lhb(&record.code).await {
                Ok(analysis) if analysis.total_score >= lhb_min_score => {
                    good_stocks.push((record.clone(), analysis));
                }
                Ok(_) => {}
                Err(e) => log::warn!("分析 {} 失败: {}", record.code, e),
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }

        if good_stocks.is_empty() {
            info!("未找到评分≥{}的股票", lhb_min_score);
            return Ok::<(Vec<(LhbRecord, LhbAnalysis)>, Vec<String>), anyhow::Error>((
                vec![],
                vec![],
            ));
        }

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

        // 过滤北交所（92 开头）
        let stock_codes: Vec<String> = good_stocks
            .iter()
            .filter(|(r, _)| !r.code.starts_with("92"))
            .map(|(r, _)| r.code.clone())
            .collect();

        if stock_codes.is_empty() {
            info!("过滤后无有效股票");
        } else {
            info!("\n开始对 {} 只股票进行完整技术分析...", stock_codes.len());
        }

        Ok((good_stocks, stock_codes))
    })?;

    if stock_codes.is_empty() {
        return Ok(());
    }

    let config = PipelineConfig {
        max_workers: get_max_workers(args),
        dry_run: args.dry_run,
        send_notification: !args.no_notify,
        single_notify: args.single_notify,
    };
    let pipeline = AnalysisPipeline::new(config)?;
    let results = runtime.block_on(pipeline.run(&stock_codes, None))?;

    info!("\n===== 龙虎榜选股分析结果 =====");
    if !results.is_empty() {
        let mut sorted = results.clone();
        sorted.sort_by(|a, b| b.sentiment_score.cmp(&a.sentiment_score));
        for r in sorted.iter() {
            let lhb_info = good_stocks
                .iter()
                .find(|(record, _)| record.code == r.code)
                .map(|(_, a)| a);
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
