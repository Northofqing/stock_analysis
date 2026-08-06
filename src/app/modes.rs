//! Registered business rules: BR-162, BR-213.
//! 三种运行模式：单次分析 / 仅大盘复盘 / 龙虎榜选股分析。

use anyhow::Result;
use chrono::Local;
use log::info;
use stock_analysis::config;
use stock_analysis::pipeline::{AnalysisPipeline, PipelineConfig};

use crate::app::get_max_workers;
use crate::cli::Args;

/// 单次分析流程（命令行默认模式）。
pub async fn run_analysis(
    stock_codes: &[String],
    args: &Args,
    macro_context: &str,
    limit_up_codes: std::collections::HashSet<String>,
) -> Result<()> {
    // 如果启用了 Multi-Agent 深度分析，则只跑深度分析
    if args.deep_analysis {
        let deep_targets: Vec<String> = match &args.stocks {
            Some(s) if !s.is_empty() => s.clone(),
            _ => stock_codes.to_vec(),
        };
        info!("模式: Multi-Agent 深度分析（共 {} 只）", deep_targets.len());
        for code in &deep_targets {
            info!("[DeepAnalysis] 开始 {}", code);
            match stock_analysis::deep_analyzer::run_and_save(code).await {
                Ok(path) => info!("[DeepAnalysis] {} 完成: {}", code, path.display()),
                Err(e) => log::error!("[DeepAnalysis] {} 失败: {:#}", code, e),
            }
        }
        return Ok(());
    }

    info!("模式: 单次分析");

    let monitor_cfg = config::get_monitor_config();

    let config = PipelineConfig {
        max_workers: get_max_workers(args),
        dry_run: args.dry_run,
        send_notification: !args.no_notify,
        single_notify: args.single_notify,
        dq_quote_stale_sec: monitor_cfg.dq_quote_stale_sec,
        dq_position_stale_sec: monitor_cfg.dq_position_stale_sec,
        dq_nav_stale_sec: monitor_cfg.dq_nav_stale_sec,
        dq_daily_stale_sec: monitor_cfg.dq_daily_stale_sec,
    };

    let pipeline = AnalysisPipeline::new(config)?.with_limit_up_codes(limit_up_codes);

    let mc = if macro_context.is_empty() {
        None
    } else {
        Some(macro_context.to_string())
    };
    let results = pipeline.run(stock_codes, mc).await?;

    if !results.is_empty() {
        info!(
            "
===== 分析结果摘要 ====="
        );
        let mut sorted_results = results;
        sorted_results.sort_by_key(|result| std::cmp::Reverse(result.sentiment_score));
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

pub async fn run_market_review_only() -> Result<()> {
    use stock_analysis::market_analyzer::MarketAnalyzer;
    use stock_analysis::notification::NotificationService;

    let (analyzer, overview) = tokio::task::spawn_blocking(|| {
        let analyzer = MarketAnalyzer::new(None)?;
        let overview = analyzer.get_market_overview()?;
        Ok::<(MarketAnalyzer, _), anyhow::Error>((analyzer, overview))
    })
    .await??;

    info!("市场概览: {:?}", overview);

    let report = analyzer.generate_template_review(&overview);
    let notifier = NotificationService::from_env();
    let filename = format!("market_review_{}.md", Local::now().format("%Y%m%d"));
    notifier.save_report_to_file(&report, Some(&filename))?;

    info!("大盘复盘完成");
    Ok(())
}

/// 产业链联动分析模式：涨停池 → 概念聚类 → 产业链上下游定位（LLM）→ 报告 + 推送。
pub async fn run_chain_analysis_mode(send_notify: bool) -> Result<()> {
    use stock_analysis::market_analyzer::MarketAnalyzer;
    use stock_analysis::notification::NotificationService;

    info!("模式: 产业链联动分析");

    let observed_at = Local::now().naive_local();
    let business_date = stock_analysis::calendar::latest_completed_trading_day_at(observed_at);

    // 涨停池获取使用阻塞 HTTP 客户端，通过 spawn_blocking 离线程
    let (_analyzer, limit_ups) = tokio::task::spawn_blocking(move || {
        let analyzer = MarketAnalyzer::new(None)?;
        let limit_ups = analyzer.get_limit_up_stocks(business_date)?;
        Ok::<(MarketAnalyzer, Vec<_>), anyhow::Error>((analyzer, limit_ups))
    })
    .await??;
    info!("今日涨停池共 {} 只", limit_ups.len());

    // 2026-08-06: 新闻收集 → AI 产业链分析。拉取主流快讯源今日新闻摘要,
    // 作为 LLM 产业链分析的宏背景 (macro_news)。任一源失败 → 显式 warn,
    // 不阻塞链分析 (聚类/落库照常, 仅 LLM 无新闻背景)。
    use stock_analysis::data_gateway::{GatewayBatch, GlobalNewsGateway, GlobalNewsProvider};
    let macro_news = match GlobalNewsGateway::new()
        .global_news(GlobalNewsProvider::Cailianpress, 20)
        .await
    {
        Ok(GatewayBatch::Available { records, .. }) if !records.is_empty() => {
            info!("[产业链] 已收集 {} 条快讯进 LLM 背景", records.len());
            Some(
                records
                    .iter()
                    .take(15)
                    .map(|r| r.title.clone())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        }
        Ok(GatewayBatch::Available { .. }) => {
            log::warn!("[产业链] 快讯批次为空, LLM 无新闻背景");
            None
        }
        Ok(GatewayBatch::VerifiedEmpty(evidence)) => {
            log::warn!("[产业链] 快讯已验证为空: {:?}", evidence.batch_id);
            None
        }
        Err(error) => {
            log::warn!("[产业链] 快讯收集失败, LLM 无新闻背景: {error}");
            None
        }
    };

    let report = stock_analysis::pipeline::chain_analysis::run_chain_analysis(
        business_date,
        limit_ups,
        macro_news,
    )
    .await?;

    let notifier = NotificationService::from_env();
    // 文件名带时段: 9:05 盘前 (business_date=昨日) / 15:30 盘后 (当日) / CLI
    // 各时段独立文件, 避免 9:05 盘前报告覆盖昨日盘后报告 (2026-08-07 接入时间线)。
    let filename = format!(
        "chain_analysis_{}_{}.md",
        business_date.format("%Y%m%d"),
        chrono::Local::now().format("%H%M")
    );
    let path = notifier.save_report_to_file(&report, Some(&filename))?;
    info!("产业链联动分析报告已保存: {}", path);

    if send_notify {
        match notifier.send(&report).await {
            Ok(true) => info!("产业链联动分析报告已推送"),
            Ok(false) => log::warn!("产业链联动分析报告推送失败（所有渠道均未成功）"),
            Err(e) => log::warn!("产业链联动分析报告推送异常: {}", e),
        }
    }
    Ok(())
}

/// 龙虎榜选股分析模式。
pub async fn run_lhb_analysis(args: &Args) -> Result<()> {
    use stock_analysis::data_gateway::{DragonTigerGateway, GatewayBatch};
    use stock_analysis::lhb_analyzer::{analyze_dragon_tiger_review, parse_dragon_tiger_date};

    let lhb_date = args.lhb_date.clone().or_else(|| {
        std::env::var("LHB_DATE")
            .ok()
            .filter(|s| !s.trim().is_empty())
    });
    let lhb_min_score = if args.lhb_min_score != 60 {
        args.lhb_min_score
    } else {
        match std::env::var("LHB_MIN_SCORE") {
            Ok(value) if !value.trim().is_empty() => value
                .parse()
                .map_err(|error| anyhow::anyhow!("LHB_MIN_SCORE 非法 {value:?}: {error}"))?,
            _ => 60,
        }
    };
    anyhow::ensure!(
        (0..=100).contains(&lhb_min_score),
        "龙虎榜最低评分必须位于 0..=100，当前={lhb_min_score}"
    );

    let trading_date = if let Some(date) = lhb_date.as_deref() {
        parse_dragon_tiger_date(date)?
    } else {
        stock_analysis::calendar::latest_completed_trading_day_at(Local::now().naive_local())
    };
    const TOP_N: usize = 10;
    info!("开始获取 {} 龙虎榜统一批次...", trading_date);
    let batch = DragonTigerGateway::new()
        .market_review(trading_date, TOP_N as u32, TOP_N)
        .await?;
    let records = match batch {
        GatewayBatch::Available { records, evidence } => {
            info!(
                "龙虎榜统一批次可用: provider={:?} source={} batch_id={} records={}",
                evidence.provider,
                evidence.source,
                evidence.batch_id,
                records.len()
            );
            records
        }
        GatewayBatch::VerifiedEmpty(evidence) => {
            info!(
                "{} 龙虎榜为来源确认空批次: provider={:?} source={} batch_id={}",
                trading_date, evidence.provider, evidence.source, evidence.batch_id
            );
            return Ok(());
        }
    };

    let mut good_stocks = Vec::new();
    for record in records {
        let analysis = analyze_dragon_tiger_review(&record)?;
        if analysis.total_score >= lhb_min_score {
            good_stocks.push((record, analysis));
        }
    }

    if good_stocks.is_empty() {
        info!("未找到评分≥{}的股票", lhb_min_score);
        return Ok(());
    }

    good_stocks.sort_by_key(|(_, analysis)| std::cmp::Reverse(analysis.total_score));
    info!("\n筛选到 {} 只优质股票:", good_stocks.len());
    for (record, analysis) in &good_stocks {
        info!(
            "  {} 龙虎榜事实评分:{} 披露:{} 显式净额:{} 正净额:{} 排名净买入:{:.0}万",
            record.code,
            analysis.total_score,
            analysis.disclosure_count,
            analysis.explicit_net_count,
            analysis.positive_net_count,
            record.ranking_net_amount_yuan / 10_000.0
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
        return Ok(());
    }

    info!("\n开始对 {} 只股票进行完整技术分析...", stock_codes.len());

    let monitor_cfg = config::get_monitor_config();
    let config = PipelineConfig {
        max_workers: get_max_workers(args),
        dry_run: args.dry_run,
        send_notification: !args.no_notify,
        single_notify: args.single_notify,
        dq_quote_stale_sec: monitor_cfg.dq_quote_stale_sec,
        dq_position_stale_sec: monitor_cfg.dq_position_stale_sec,
        dq_nav_stale_sec: monitor_cfg.dq_nav_stale_sec,
        dq_daily_stale_sec: monitor_cfg.dq_daily_stale_sec,
    };
    let pipeline = AnalysisPipeline::new(config)?;
    let results = pipeline.run(&stock_codes, None).await?;

    info!("\n===== 龙虎榜选股分析结果 =====");
    if !results.is_empty() {
        let mut sorted = results;
        sorted.sort_by_key(|result| std::cmp::Reverse(result.sentiment_score));
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
