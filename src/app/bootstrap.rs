//! Registered business rules: BR-162, BR-213.
//! 启动前处理：配置校验、自选股列表装配（含宏观 AI 推荐 / 龙虎榜 / 涨停 / 持仓）。

use anyhow::Result;
use log::{error, info};
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;

use crate::cli::Args;

/// 6 位 A 股代码（沪深主板/中小创/科创板）：以 0/3/6 开头。
static STOCK_CODE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b([036]\d{5})\b").expect("内置股票代码正则不应失败"));

/// 启动前配置校验：检查 AI 模型与通知渠道等关键配置，
/// 任一项不合法即打印明确提示并立即退出（exit code 1）。
pub fn validate_startup_config() {
    use stock_analysis::notification::NotificationConfig;

    let mut errors: Vec<String> = Vec::new();

    // AI 模型：至少配置一个有效 Key
    let has_any_ai = ["GEMINI_API_KEY", "DEEPSEEK_API_KEY", "DOUBAO_API_KEY"]
        .iter()
        .any(|k| {
            std::env::var(k)
                .ok()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
        });
    if !has_any_ai {
        errors.push(
            "未配置任何 AI 模型：请在 .env 至少填写 GEMINI_API_KEY / DEEPSEEK_API_KEY / DOUBAO_API_KEY 中的一个"
                .to_string()
        );
    }

    // 通知渠道一致性校验
    errors.extend(NotificationConfig::from_env().validate());

    if errors.is_empty() {
        return;
    }

    let env_path = std::path::Path::new(".env")
        .canonicalize()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "./.env".to_string());

    error!("============================================================");
    error!("❌ 启动配置校验失败，已中止运行。请按以下提示修改 .env 后重试：");
    error!("------------------------------------------------------------");
    for (i, e) in errors.iter().enumerate() {
        error!("  {}. {}", i + 1, e);
    }
    error!("------------------------------------------------------------");
    error!(".env 路径: {}", env_path);
    error!("============================================================");
    std::process::exit(1);
}

/// 组装待分析股票列表。
///
/// 来源（去重合并）：
/// 1. 命令行 `--stocks` 或环境变量 `STOCK_LIST`
/// 2. 宏观 AI 推荐
/// 3. 当日龙虎榜净买入 Top 10（过滤北交所）
/// 4. 当日涨停股票（过滤北交所与 ST）
/// 5. 数据库中持仓中的股票
///
/// 返回 `(stock_codes, limit_up_codes, macro_news_context)`。
pub async fn build_stock_list(args: &Args) -> Result<(Vec<String>, HashSet<String>, String)> {
    // 1. 自选股基础列表
    let mut stock_codes: Vec<String> = if let Some(ref stocks) = args.stocks {
        stocks.clone()
    } else {
        std::env::var("STOCK_LIST")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect()
    };

    // 2. 宏观 AI 推荐（受 MACRO_AI_ENABLED 控制，默认开启）
    // 若使用 --deep-analysis 模式，则强制关闭扩展，只分析输入的票
    let macro_ai_enabled = if args.deep_analysis {
        false
    } else {
        std::env::var("MACRO_AI_ENABLED")
            .map(|v| v.to_lowercase() != "false")
            .unwrap_or(true)
    };
    let macro_news_context = if macro_ai_enabled {
        let (extra_codes, macro_text) = fetch_macro_recommended_codes().await;
        if !extra_codes.is_empty() {
            let before = stock_codes.len();
            for code in &extra_codes {
                if !stock_codes.contains(code) {
                    stock_codes.push(code.clone());
                }
            }
            info!(
                "📈 宏观AI推荐 {} 只，新增追加 {} 只（去重后）",
                extra_codes.len(),
                stock_codes.len() - before
            );
        }
        macro_text
    } else {
        info!("⚙️ MACRO_AI_ENABLED=false：跳过宏观 AI 新闻分析与推荐");
        String::new()
    };

    // 3. 龙虎榜 Top 10（受 LHB_APPEND_ENABLED 控制，默认开启）
    let lhb_append_enabled = if args.deep_analysis {
        false
    } else {
        std::env::var("LHB_APPEND_ENABLED")
            .map(|v| v.to_lowercase() != "false")
            .unwrap_or(true)
    };
    if lhb_append_enabled {
        append_lhb_top10(&mut stock_codes).await?;
    } else {
        info!("⚙️ LHB_APPEND_ENABLED=false：跳过龙虎榜 Top10 追加");
    }

    // 4. 涨停股票（受 LIMIT_UP_APPEND_ENABLED 控制，默认开启）
    let limit_up_append_enabled = if args.deep_analysis {
        false
    } else {
        std::env::var("LIMIT_UP_APPEND_ENABLED")
            .map(|v| v.to_lowercase() != "false")
            .unwrap_or(true)
    };
    let limit_up_codes = if limit_up_append_enabled {
        let observed_at = chrono::Local::now().naive_local();
        let trading_date = match stock_analysis::calendar::current_session() {
            stock_analysis::calendar::MarketSession::Auction
            | stock_analysis::calendar::MarketSession::Morning
            | stock_analysis::calendar::MarketSession::LunchBreak
            | stock_analysis::calendar::MarketSession::Afternoon
            | stock_analysis::calendar::MarketSession::AfterHours => observed_at.date(),
            stock_analysis::calendar::MarketSession::Closed => {
                stock_analysis::calendar::latest_completed_trading_day_at(observed_at)
            }
        };
        let mut owned_codes = std::mem::take(&mut stock_codes);
        let (resolved_codes, limit_up_codes) = tokio::task::spawn_blocking(move || {
            let limit_up_codes = append_limit_up(&mut owned_codes, trading_date);
            (owned_codes, limit_up_codes)
        })
        .await
        .map_err(|error| anyhow::anyhow!("BR-213 涨停池 blocking worker 失败: {error}"))?;
        stock_codes = resolved_codes;
        limit_up_codes
    } else {
        info!("⚙️ LIMIT_UP_APPEND_ENABLED=false：跳过当日涨停追加");
        HashSet::new()
    };

    // 5. 持仓股票（受 POSITION_TRACKING_ENABLED 控制，默认开启）
    let position_tracking_enabled = std::env::var("POSITION_TRACKING_ENABLED")
        .map(|v| v.to_lowercase() != "false")
        .unwrap_or(true);
    if position_tracking_enabled {
        append_open_positions(&mut stock_codes);
    } else {
        info!("⚙️ POSITION_TRACKING_ENABLED=false：跳过持仓追加与持仓跟踪");
    }

    // 6. 过滤退市股票（默认开启，可通过 STOCK_FILTER_DELISTED=false 关闭）
    filter_delisted_stocks(&mut stock_codes).await?;

    if stock_codes.is_empty() {
        info!("⚠️ 未配置自选股列表且宏观AI未推荐股票，将仅执行大盘复盘");
    }

    Ok((stock_codes, limit_up_codes, macro_news_context))
}

#[derive(Debug)]
struct DelistedFilterProjection {
    retained_codes: Vec<String>,
    removed: Vec<(String, String)>,
    evidence: stock_analysis::data_gateway::BatchEvidence,
}

fn project_delisted_filter(
    requested_codes: &[String],
    batch: stock_analysis::data_gateway::GatewayBatch<
        stock_analysis::data_gateway::market_capabilities::MarketSecurityIdentity,
    >,
) -> std::result::Result<DelistedFilterProjection, String> {
    use stock_analysis::data_gateway::GatewayBatch;

    let (records, evidence) = match batch {
        GatewayBatch::Available { records, evidence } if !records.is_empty() => (records, evidence),
        GatewayBatch::Available { evidence, .. } | GatewayBatch::VerifiedEmpty(evidence) => {
            return Err(format!(
                "证券身份批次没有可用于退市判定的记录: source={} batch_id={}",
                evidence.source, evidence.batch_id
            ));
        }
    };
    if records.len() != requested_codes.len() {
        return Err(format!(
            "证券身份批次不完整: requested={} actual={} source={} batch_id={}",
            requested_codes.len(),
            records.len(),
            evidence.source,
            evidence.batch_id
        ));
    }

    let mut retained_codes = Vec::with_capacity(records.len());
    let mut removed = Vec::new();
    for (requested_code, metadata) in requested_codes.iter().zip(records) {
        if metadata.code != *requested_code
            || metadata.provider != evidence.provider
            || metadata.batch_id != evidence.batch_id
        {
            return Err(format!(
                "证券身份/证据不匹配: requested={} actual={} source={} batch_id={}",
                requested_code, metadata.code, evidence.source, evidence.batch_id
            ));
        }
        if metadata.name.trim().is_empty() {
            return Err(format!(
                "证券身份名称缺失: code={} source={} batch_id={}",
                requested_code, evidence.source, evidence.batch_id
            ));
        }
        if is_delisted_name(&metadata.name) {
            removed.push((requested_code.clone(), metadata.name));
        } else {
            retained_codes.push(requested_code.clone());
        }
    }

    Ok(DelistedFilterProjection {
        retained_codes,
        removed,
        evidence,
    })
}

async fn filter_delisted_stocks(stock_codes: &mut Vec<String>) -> Result<()> {
    let filter_enabled = std::env::var("STOCK_FILTER_DELISTED")
        .map(|v| v.to_lowercase() != "false")
        .unwrap_or(true);
    if !filter_enabled {
        info!("⚙️ STOCK_FILTER_DELISTED=false：跳过退市股票过滤");
        return Ok(());
    }
    if stock_codes.is_empty() {
        return Ok(());
    }

    let before = stock_codes.len();
    let batch = stock_analysis::data_gateway::MarketCapabilitiesGateway::new()
        .security_identities(stock_codes)
        .await
        .map_err(|error| anyhow::anyhow!("退市过滤证券身份数据不可用: {error}"))?;
    let projection = project_delisted_filter(stock_codes, batch).map_err(anyhow::Error::msg)?;
    info!(
        "[BR-164] 退市过滤采用统一证券身份批次: provider={:?} source={} batch_id={} records={}",
        projection.evidence.provider,
        projection.evidence.source,
        projection.evidence.batch_id,
        before
    );

    for (code, name) in &projection.removed {
        info!("🚫 过滤退市票: {}({})", name, code);
    }
    *stock_codes = projection.retained_codes;
    if !projection.removed.is_empty() {
        info!(
            "🚫 退市过滤完成：移除 {} 只，剩余 {} 只",
            before - stock_codes.len(),
            stock_codes.len()
        );
    }
    Ok(())
}

fn is_delisted_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.contains("退市") || trimmed.starts_with('退') || trimmed.contains("终止上市")
}

async fn append_lhb_top10(stock_codes: &mut Vec<String>) -> Result<()> {
    use stock_analysis::data_gateway::{DragonTigerGateway, GatewayBatch};

    const TOP_N: usize = 10;
    let trading_date = stock_analysis::calendar::latest_completed_trading_day_at(
        chrono::Local::now().naive_local(),
    );
    let batch = DragonTigerGateway::new()
        .market_review(trading_date, TOP_N as u32, TOP_N)
        .await?;
    match batch {
        GatewayBatch::Available { records, evidence } => {
            info!(
                "🐉 龙虎榜统一批次: date={} provider={:?} source={} batch_id={} records={}",
                trading_date,
                evidence.provider,
                evidence.source,
                evidence.batch_id,
                records.len()
            );
            let before = stock_codes.len();
            for record in records {
                if record.code.starts_with("92") {
                    continue; // 过滤北交所
                }
                if !stock_codes.contains(&record.code) {
                    info!(
                        "🐉 龙虎榜追加: {} 排名净买入{:.0}万",
                        record.code,
                        record.ranking_net_amount_yuan / 10_000.0
                    );
                    stock_codes.push(record.code);
                }
            }
            info!(
                "🐉 龙虎榜Top{} 新增追加 {} 只（去重后）",
                TOP_N,
                stock_codes.len() - before
            );
        }
        GatewayBatch::VerifiedEmpty(evidence) => info!(
            "📋 {} 龙虎榜为来源确认空批次: provider={:?} source={} batch_id={}",
            trading_date, evidence.provider, evidence.source, evidence.batch_id
        ),
    }
    Ok(())
}

fn append_limit_up(
    stock_codes: &mut Vec<String>,
    trading_date: chrono::NaiveDate,
) -> HashSet<String> {
    use stock_analysis::market_analyzer::MarketAnalyzer;

    let mut set = HashSet::new();
    let analyzer = match MarketAnalyzer::new(None) {
        Ok(a) => a,
        Err(e) => {
            info!("⚠️ 创建市场分析器失败: {}", e);
            return set;
        }
    };
    match analyzer.get_limit_up_stocks(trading_date) {
        Ok(stocks) if !stocks.is_empty() => {
            let before = stock_codes.len();
            for stock in &stocks {
                set.insert(stock.code.clone());
                if !stock_codes.contains(&stock.code) {
                    info!(
                        "🔥 涨停追加: {}({}) 涨幅{:.2}%",
                        stock.name, stock.code, stock.change_pct
                    );
                    stock_codes.push(stock.code.clone());
                }
            }
            info!(
                "🔥 当日涨停 {} 只，新增追加 {} 只（去重后）",
                stocks.len(),
                stock_codes.len() - before
            );
        }
        Ok(_) => info!("📋 今日暂无涨停股票"),
        Err(e) => info!("⚠️ 获取涨停股票失败（不影响正常分析）: {}", e),
    }
    set
}

fn append_open_positions(stock_codes: &mut Vec<String>) {
    use stock_analysis::database::DatabaseManager;

    let Some(db) = DatabaseManager::try_get() else {
        return;
    };
    match db.get_all_open_positions() {
        Ok(positions) if !positions.is_empty() => {
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
            info!(
                "💰 持仓中 {} 只，新增追加 {} 只（去重后）",
                positions.len(),
                stock_codes.len() - before
            );
        }
        Ok(_) => {}
        Err(e) => info!("⚠️ 查询持仓数据失败（不影响正常分析）: {}", e),
    }
}

/// 通过宏观新闻 AI 分析，返回 (推荐的 A 股代码列表, 宏观新闻全文)。
///
/// 宏观新闻全文会由调用方传递给 pipeline，避免重复搜索。
pub(crate) async fn fetch_macro_recommended_codes() -> (Vec<String>, String) {
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
        let guard = get_analyzer().lock().await;
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
            info!(
                "========== 宏观驱动 A 股推荐 ==========\n{}\n========================================",
                rec_text
            );
            save_macro_report(&mc, &rec_text);
            let codes = extract_stock_codes(&rec_text);
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

fn save_macro_report(macro_ctx: &str, rec_text: &str) {
    let date_str = chrono::Local::now().format("%Y%m%d").to_string();
    let filename = format!("reports/macro_recommendations_{}.md", date_str);
    let content = format!(
        "# 📈 宏观驱动 A 股推荐报告\n\n**生成时间**: {}\n\n---\n\n## 今日宏观背景\n\n{}\n\n---\n\n{}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        macro_ctx,
        rec_text
    );
    if let Err(e) = std::fs::write(&filename, &content) {
        log::warn!("宏观推荐报告保存失败: {}", e);
    } else {
        info!("✓ 宏观推荐报告已保存: {}", filename);
    }
}

fn extract_stock_codes(rec_text: &str) -> Vec<String> {
    // 优先从【推荐代码】行提取（更可靠），回退到全文正则
    let code_line_text = rec_text
        .lines()
        .find(|line| line.contains("【推荐代码】"))
        .unwrap_or(rec_text);
    let mut codes: Vec<String> = STOCK_CODE_RE
        .captures_iter(code_line_text)
        .map(|cap| cap[1].to_string())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if codes.is_empty() {
        codes = STOCK_CODE_RE
            .captures_iter(rec_text)
            .map(|cap| cap[1].to_string())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
    }
    codes.sort();
    codes
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use magic_market_core::ProviderId;
    use stock_analysis::data_gateway::market_capabilities::MarketSecurityIdentity;
    use stock_analysis::data_gateway::{BatchEvidence, GatewayBatch};

    fn metadata_evidence() -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_magic_tdx_metadata".to_string(),
            source_at: Some("2026-07-26T01:00:00Z".to_string()),
            observed_at: "2026-07-26T01:00:01Z".to_string(),
            batch_id: "TEST_CODE_metadata_batch".to_string(),
        }
    }

    fn security_metadata(code: &str, name: &str) -> MarketSecurityIdentity {
        let source_at = chrono::DateTime::parse_from_rfc3339("2026-07-26T01:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        MarketSecurityIdentity {
            code: code.to_string(),
            name: name.to_string(),
            is_st: false,
            source_at,
            observed_at: source_at + chrono::Duration::seconds(1),
            provider: ProviderId::Tdx,
            batch_id: "TEST_CODE_metadata_batch".to_string(),
        }
    }

    #[test]
    fn stock_code_extraction_prefers_registered_line_then_falls_back_and_deduplicates() {
        assert_eq!(
            extract_stock_codes(
                "正文提到 600000\n【推荐代码】 300001, 000002, 300001\n尾部 688001"
            ),
            vec!["000002", "300001"]
        );
        assert_eq!(
            extract_stock_codes("没有登记行，正文含 600000 / 688001 / 600000 / 920001"),
            vec!["600000", "688001"]
        );
        assert!(extract_stock_codes("没有合法证券身份 12345 TEST_CODE_000001").is_empty());
    }

    #[test]
    fn delisted_name_detection_covers_all_registered_labels() {
        assert!(is_delisted_name("退市测试"));
        assert!(is_delisted_name("退测试"));
        assert!(is_delisted_name("测试终止上市"));
        assert!(!is_delisted_name("普通测试股"));
    }

    #[test]
    fn br164_delisted_filter_uses_complete_available_metadata_and_retains_evidence() {
        let requested = vec![
            "TEST_CODE_600001".to_string(),
            "TEST_CODE_600002".to_string(),
        ];
        let projection = project_delisted_filter(
            &requested,
            GatewayBatch::Available {
                records: vec![
                    security_metadata("TEST_CODE_600001", "普通测试股"),
                    security_metadata("TEST_CODE_600002", "退市测试"),
                ],
                evidence: metadata_evidence(),
            },
        )
        .unwrap();

        assert_eq!(
            projection.retained_codes,
            vec!["TEST_CODE_600001".to_string()]
        );
        assert_eq!(
            projection.removed,
            vec![("TEST_CODE_600002".to_string(), "退市测试".to_string())]
        );
        assert_eq!(projection.evidence.batch_id, "TEST_CODE_metadata_batch");
    }

    #[test]
    fn br164_delisted_filter_rejects_verified_empty_or_partial_metadata() {
        let requested = vec!["TEST_CODE_600001".to_string()];
        assert!(project_delisted_filter(
            &requested,
            GatewayBatch::<MarketSecurityIdentity>::VerifiedEmpty(metadata_evidence())
        )
        .is_err());
        assert!(project_delisted_filter(
            &requested,
            GatewayBatch::Available {
                records: Vec::new(),
                evidence: metadata_evidence(),
            },
        )
        .is_err());
    }

    #[tokio::test]
    #[serial_test::serial(bootstrap_env)]
    async fn deep_analysis_stock_list_uses_only_explicit_codes_without_external_extensions() {
        let keys = ["POSITION_TRACKING_ENABLED", "STOCK_FILTER_DELISTED"];
        let previous: Vec<_> = keys
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        std::env::set_var("POSITION_TRACKING_ENABLED", "false");
        std::env::set_var("STOCK_FILTER_DELISTED", "false");
        let args = Args::parse_from([
            "stock_analysis",
            "--deep-analysis",
            "--stocks",
            "TEST_CODE_000001,TEST_CODE_600000",
        ]);

        let (codes, limit_up, macro_context) = build_stock_list(&args).await.unwrap();

        for (key, value) in previous {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        assert_eq!(
            codes,
            vec![
                "TEST_CODE_000001".to_string(),
                "TEST_CODE_600000".to_string()
            ]
        );
        assert!(limit_up.is_empty());
        assert!(macro_context.is_empty());
    }
}
