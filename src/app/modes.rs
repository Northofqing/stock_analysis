//! Registered business rules: BR-162, BR-213.
//! 三种运行模式：单次分析 / 仅大盘复盘 / 龙虎榜选股分析。

use anyhow::{Context, Result};
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
    let outcome = pipeline.run(stock_codes, mc).await?;
    outcome.log_completion();
    let completion = outcome.ensure_cli_success();
    let results = outcome.results;

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

    completion
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
    run_chain_analysis_mode_with_send_guard(send_notify, None, |_| Ok(())).await
}

/// The scheduler uses the guard to record a durable attempt after the report is
/// saved and immediately before the first external notification call. CLI runs
/// retain their invocation-local behavior.
pub async fn run_chain_analysis_mode_with_send_guard(
    send_notify: bool,
    scheduled_filename: Option<&str>,
    before_send: impl FnOnce(&str) -> Result<()>,
) -> Result<()> {
    run_chain_analysis_mode_with_observation(send_notify, scheduled_filename, before_send)
        .await?
        .legacy_result
}

/// Data retained for a scheduled, read-only observation. It grants no delivery authority.
pub(super) struct ChainDeliveryEnvelope {
    pub prepared: stock_analysis::pipeline::chain_analysis::preparation::PreparedChainAnalysis,
    pub acquisition: Option<super::chain_acquisition::ChainAcquisitionEvidence>,
    pub send_attempted: bool,
    pub report_input: Vec<u8>,
    pub legacy_result: Result<()>,
}

pub(super) async fn run_chain_analysis_mode_with_observation(
    send_notify: bool,
    scheduled_filename: Option<&str>,
    before_send: impl FnOnce(&str) -> Result<()>,
) -> Result<ChainDeliveryEnvelope> {
    use stock_analysis::market_analyzer::MarketAnalyzer;
    use stock_analysis::notification::NotificationService;

    info!("模式: 产业链联动分析");

    // 2026-08-06: 新闻收集 → AI 产业链分析。拉取主流快讯源今日新闻摘要,
    // 作为 LLM 产业链分析的宏背景 (macro_news)。任一源失败 → 显式 warn,
    // 不阻塞链分析 (聚类/落库照常, 仅 LLM 无新闻背景)。
    use stock_analysis::data_gateway::{GlobalNewsGateway, GlobalNewsProvider};
    let observed_at = Local::now().fixed_offset();
    let (prepared, acquisition) = super::chain_acquisition::prepare_with_acquisition(
        observed_at,
        |business_date| async move {
            // The blocking gateway and its audit run once on their existing worker.
            let observation = tokio::task::spawn_blocking(move || {
                let analyzer = MarketAnalyzer::new(None)?;
                analyzer.get_limit_up_observation(business_date)
            })
            .await??;
            Ok(super::chain_acquisition::ChainLimitAcquisition::from_observation(observation))
        },
        || async {
            GlobalNewsGateway::new()
                .global_news(GlobalNewsProvider::Cailianpress, 20)
                .await
        },
        |business_date, limit_ups, macro_news| async move {
            stock_analysis::pipeline::chain_analysis::preparation::prepare_chain_analysis(
                business_date,
                limit_ups,
                macro_news,
            )
            .await
        },
    )
    .await?;
    let business_date = acquisition.business_date;

    let notifier = std::sync::Arc::new(NotificationService::from_env());
    let save_notifier = notifier.clone();
    let available = notifier.is_available();
    // 文件名带时段: 9:05 盘前 (business_date=昨日) / 15:30 盘后 (当日) / CLI
    // 各时段独立文件, 避免 9:05 盘前报告覆盖昨日盘后报告 (2026-08-07 接入时间线)。
    let default_filename = format!(
        "chain_analysis_{}_{}.md",
        business_date.format("%Y%m%d"),
        chrono::Local::now().format("%H%M")
    );
    let filename = scheduled_filename.unwrap_or(&default_filename);
    deliver_prepared(
        prepared,
        Some(acquisition),
        move |report| save_notifier.save_report_to_file(report, Some(filename)),
        available,
        before_send,
        move |report| Box::pin(async move { notifier.send(report).await }),
        send_notify,
    )
    .await
}

async fn deliver_prepared<S, G, T>(
    prepared: stock_analysis::pipeline::chain_analysis::preparation::PreparedChainAnalysis,
    acquisition: Option<super::chain_acquisition::ChainAcquisitionEvidence>,
    save: S,
    available: bool,
    before_send: G,
    send: T,
    send_notify: bool,
) -> Result<ChainDeliveryEnvelope>
where
    S: FnOnce(&str) -> Result<String>,
    G: FnOnce(&str) -> Result<()>,
    T: for<'a> FnOnce(&'a str) -> futures::future::LocalBoxFuture<'a, Result<bool>>,
{
    let path = save(prepared.report())?;
    info!("产业链联动分析报告已保存: {}", path);
    let mut envelope = ChainDeliveryEnvelope {
        report_input: prepared.report().as_bytes().to_vec(),
        prepared,
        acquisition,
        send_attempted: false,
        legacy_result: Ok(()),
    };
    if send_notify {
        if !available {
            envelope.legacy_result = Err(anyhow::anyhow!("产业链联动分析报告没有可用通知渠道"));
        } else if let Err(error) = before_send(&path) {
            envelope.legacy_result = Err(error);
        } else {
            envelope.send_attempted = true;
            envelope.legacy_result =
                require_chain_notification_success(send(envelope.prepared.report()).await);
        }
    }
    Ok(envelope)
}

fn require_chain_notification_success(result: Result<bool>) -> Result<()> {
    match result.context("产业链联动分析报告推送异常")? {
        true => {
            info!("产业链联动分析报告已推送");
            Ok(())
        }
        false => anyhow::bail!("产业链联动分析报告推送失败（所有渠道均未成功）"),
    }
}

#[cfg(test)]
mod tests_chain_delivery {
    use super::{deliver_prepared, require_chain_notification_success, ChainDeliveryEnvelope};
    use crate::app::chain_schedule::{finish_scheduled_delivery, ChainPhase};
    use crate::app::chain_shadow_input::{observe, test_prepared};
    use chrono::NaiveDate;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    #[test]
    fn chain_send_rejects_weak_failure() {
        assert!(require_chain_notification_success(Ok(true)).is_ok());
        assert!(require_chain_notification_success(Ok(false)).is_err());
        assert!(
            require_chain_notification_success(Err(anyhow::anyhow!("TEST_CODE_SEND_DOWN")))
                .is_err()
        );
    }

    async fn scripted_delivery(
        guard_ok: bool,
        send_ok: bool,
        observer_ok: bool,
    ) -> (Vec<&'static str>, String, Vec<u8>, bool, bool) {
        let date = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let preparations = Rc::new(Cell::new(0));
        let prepared = test_prepared(date, preparations.clone()).await;
        let expected_report = prepared.report().to_owned();
        let events = Rc::new(RefCell::new(Vec::new()));
        let saved = Rc::new(RefCell::new(Vec::new()));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let envelope = deliver_prepared(
            prepared,
            None,
            {
                let events = events.clone();
                let saved = saved.clone();
                move |report| {
                    events.borrow_mut().push("save");
                    saved.borrow_mut().extend_from_slice(report.as_bytes());
                    Ok("test-report.md".into())
                }
            },
            true,
            {
                let events = events.clone();
                move |_| {
                    events.borrow_mut().push("guard");
                    if guard_ok {
                        Ok(())
                    } else {
                        anyhow::bail!("guard failed")
                    }
                }
            },
            {
                let events = events.clone();
                let sent = sent.clone();
                move |report| {
                    Box::pin(async move {
                        events.borrow_mut().push("send");
                        sent.borrow_mut().extend_from_slice(report.as_bytes());
                        Ok(send_ok)
                    })
                }
            },
            true,
        )
        .await
        .unwrap();
        assert_eq!(preparations.get(), 1);
        assert_eq!(&*saved.borrow(), expected_report.as_bytes());
        let attempted = envelope.send_attempted;
        assert_eq!(&envelope.report_input, &*saved.borrow());
        if envelope.send_attempted {
            assert_eq!(&envelope.report_input, &*sent.borrow());
        }
        let result = finish_scheduled_delivery(
            envelope,
            ChainPhase::Postclose,
            date,
            {
                let events = events.clone();
                move || {
                    events.borrow_mut().push("mark");
                    Ok(())
                }
            },
            {
                let events = events.clone();
                move |phase, date, prepared, report_input, acquisition| {
                    events.borrow_mut().push("observe");
                    if observer_ok {
                        observe(phase, date, prepared, report_input, acquisition)
                    } else {
                        anyhow::bail!("observer failed")
                    }
                }
            },
        );
        let events = events.borrow().clone();
        let sent = sent.borrow().clone();
        (events, expected_report, sent, attempted, result.is_ok())
    }

    #[tokio::test]
    async fn one_preparation_and_exact_order_and_report_bytes() {
        let (events, report, sent, attempted, success) = scripted_delivery(true, true, true).await;
        assert_eq!(events, ["save", "guard", "send", "mark", "observe"]);
        assert_eq!(sent, report.as_bytes());
        assert!(attempted && success);
    }

    #[tokio::test]
    async fn send_failure_is_observed_and_guard_failure_is_not() {
        let (events, _, _, attempted, success) = scripted_delivery(true, false, true).await;
        assert_eq!(events, ["save", "guard", "send", "observe"]);
        assert!(attempted && !success);
        let (events, _, sent, attempted, success) = scripted_delivery(false, true, true).await;
        assert_eq!(events, ["save", "guard"]);
        assert!(sent.is_empty() && !attempted && !success);
    }

    #[tokio::test]
    async fn observer_failure_preserves_legacy_success_and_failure() {
        let (events, _, _, _, success) = scripted_delivery(true, true, false).await;
        assert_eq!(events, ["save", "guard", "send", "mark", "observe"]);
        assert!(success);
        let (events, _, _, _, success) = scripted_delivery(true, false, false).await;
        assert_eq!(events, ["save", "guard", "send", "observe"]);
        assert!(!success);
    }

    #[tokio::test]
    async fn failed_weak_mark_is_observed_without_changing_its_error() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let prepared = test_prepared(date, Rc::new(Cell::new(0))).await;
        let envelope = ChainDeliveryEnvelope {
            report_input: prepared.report().as_bytes().to_vec(),
            prepared,
            acquisition: None,
            send_attempted: true,
            legacy_result: Ok(()),
        };
        let events = RefCell::new(Vec::new());
        let error = finish_scheduled_delivery(
            envelope,
            ChainPhase::Postclose,
            date,
            || {
                events.borrow_mut().push("mark");
                anyhow::bail!("weak mark failed")
            },
            |phase, date, prepared, input, acquisition| {
                events.borrow_mut().push("observe");
                observe(phase, date, prepared, input, acquisition)
            },
        )
        .unwrap_err();
        assert_eq!(&*events.borrow(), &["mark", "observe"]);
        assert_eq!(error.to_string(), "weak mark failed");
    }
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
    let outcome = pipeline.run(&stock_codes, None).await?;
    outcome.log_completion();
    let completion = outcome.ensure_cli_success();
    let results = outcome.results;

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
    completion
}
