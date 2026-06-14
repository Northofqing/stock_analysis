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
    let runtime = tokio::runtime::Runtime::new()?;

    // 如果启用了 Multi-Agent 深度分析，则只跑深度分析
    if args.deep_analysis {
        let deep_targets: Vec<String> = match &args.stocks {
            Some(s) if !s.is_empty() => s.clone(),
            _ => stock_codes.to_vec(),
        };
        info!(
            "模式: Multi-Agent 深度分析（共 {} 只）",
            deep_targets.len()
        );
        runtime.block_on(async {
            for code in &deep_targets {
                info!("[DeepAnalysis] 开始 {}", code);
                match stock_analysis::deep_analyzer::run_and_save(code).await {
                    Ok(path) => info!("[DeepAnalysis] {} 完成: {}", code, path.display()),
                    Err(e) => log::error!("[DeepAnalysis] {} 失败: {:#}", code, e),
                }
            }
        });
        return Ok(());
    }

    info!("模式: 单次分析");

    let config = PipelineConfig {
        max_workers: get_max_workers(args),
        dry_run: args.dry_run,
        send_notification: !args.no_notify,
        single_notify: args.single_notify,
    };

    let pipeline = AnalysisPipeline::new(config)?.with_limit_up_codes(limit_up_codes);

    let mc = if macro_context.is_empty() {
        None
    } else {
        Some(macro_context.to_string())
    };
    let results = runtime.block_on(pipeline.run(stock_codes, mc))?;

    if !results.is_empty() {
        info!("
===== 分析结果摘要 =====");
        let mut sorted_results = results;
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

/// 产业链联动分析模式：涨停池 → 概念聚类 → 产业链上下游定位（LLM）→ 报告 + 推送。
pub fn run_chain_analysis_mode(send_notify: bool) -> Result<()> {
    use stock_analysis::market_analyzer::MarketAnalyzer;
    use stock_analysis::notification::NotificationService;

    info!("模式: 产业链联动分析");

    // 涨停池获取使用阻塞 HTTP 客户端，须在进入 tokio 运行时之前完成
    let analyzer = MarketAnalyzer::new(None)?;
    let limit_ups = analyzer.get_limit_up_stocks()?;
    info!("今日涨停池共 {} 只", limit_ups.len());

    let runtime = tokio::runtime::Runtime::new()?;
    let report = runtime.block_on(
        stock_analysis::pipeline::chain_analysis::run_chain_analysis(limit_ups, None),
    )?;

    let notifier = NotificationService::from_env();
    let filename = format!("chain_analysis_{}.md", Local::now().format("%Y%m%d"));
    let path = notifier.save_report_to_file(&report, Some(&filename))?;
    info!("产业链联动分析报告已保存: {}", path);

    if send_notify {
        match runtime.block_on(notifier.send(&report)) {
            Ok(true) => info!("产业链联动分析报告已推送"),
            Ok(false) => log::warn!("产业链联动分析报告推送失败（所有渠道均未成功）"),
            Err(e) => log::warn!("产业链联动分析报告推送异常: {}", e),
        }
    }
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

        let total = unique_lhb.len();
        let mut good_stocks = Vec::new();
        for (i, record) in unique_lhb.into_iter().enumerate() {
            if i > 0 && i % 10 == 0 {
                info!("已处理 {}/{} 只股票", i, total);
            }
            match fetcher.analyze_stock_lhb(&record.code).await {
                Ok(analysis) if analysis.total_score >= lhb_min_score => {
                    good_stocks.push((record, analysis));
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
        let mut sorted = results;
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
