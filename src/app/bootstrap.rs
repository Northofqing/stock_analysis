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
    let has_any_ai = ["GEMINI_API_KEY", "OPENAI_API_KEY", "DOUBAO_API_KEY"]
        .iter()
        .any(|k| {
            std::env::var(k)
                .ok()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
        });
    if !has_any_ai {
        errors.push(
            "未配置任何 AI 模型：请在 .env 至少填写 GEMINI_API_KEY / OPENAI_API_KEY / DOUBAO_API_KEY 中的一个".to_string()
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
pub fn build_stock_list(args: &Args) -> Result<(Vec<String>, HashSet<String>, String)> {
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
        let runtime = tokio::runtime::Runtime::new()?;
        let (extra_codes, macro_text) = runtime.block_on(fetch_macro_recommended_codes());
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

    // 2.5 板块共振引擎（涨幅榜 ∩ 主力净流入榜 ∩ 宏观新闻）
    let sector_resonance_enabled = if args.deep_analysis {
        false
    } else {
        std::env::var("SECTOR_RESONANCE_ENABLED")
            .map(|v| v.to_lowercase() != "false")
            .unwrap_or(true)
    };
    if sector_resonance_enabled {
        append_sector_resonance(&mut stock_codes, &macro_news_context);
    } else {
        info!("⚙️ SECTOR_RESONANCE_ENABLED=false：跳过板块共振追加");
    }

    // 3. 龙虎榜 Top 10（受 LHB_APPEND_ENABLED 控制，默认开启）
    let lhb_append_enabled = if args.deep_analysis {
        false
    } else {
        std::env::var("LHB_APPEND_ENABLED")
            .map(|v| v.to_lowercase() != "false")
            .unwrap_or(true)
    };
    if lhb_append_enabled {
        append_lhb_top10(&mut stock_codes)?;
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
        append_limit_up(&mut stock_codes)
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
    filter_delisted_stocks(&mut stock_codes);

    if stock_codes.is_empty() {
        info!("⚠️ 未配置自选股列表且宏观AI未推荐股票，将仅执行大盘复盘");
    }

    Ok((stock_codes, limit_up_codes, macro_news_context))
}

fn filter_delisted_stocks(stock_codes: &mut Vec<String>) {
    let filter_enabled = std::env::var("STOCK_FILTER_DELISTED")
        .map(|v| v.to_lowercase() != "false")
        .unwrap_or(true);
    if !filter_enabled {
        info!("⚙️ STOCK_FILTER_DELISTED=false：跳过退市股票过滤");
        return;
    }

    use stock_analysis::data_provider::DataFetcherManager;

    let fetcher = match DataFetcherManager::new() {
        Ok(f) => f,
        Err(e) => {
            info!("⚠️ 初始化数据获取器失败，跳过退市过滤: {}", e);
            return;
        }
    };

    let before = stock_codes.len();
    let mut removed: Vec<(String, String)> = Vec::new();
    stock_codes.retain(|code| match fetcher.get_stock_name(code) {
        Some(name) if is_delisted_name(&name) => {
            removed.push((code.clone(), name));
            false
        }
        _ => true,
    });

    if removed.is_empty() {
        return;
    }

    for (code, name) in &removed {
        info!("🚫 过滤退市票: {}({})", name, code);
    }
    info!(
        "🚫 退市过滤完成：移除 {} 只，剩余 {} 只",
        before - stock_codes.len(),
        stock_codes.len()
    );
}

fn is_delisted_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.contains("退市") || trimmed.starts_with('退') || trimmed.contains("终止上市")
}

fn append_lhb_top10(stock_codes: &mut Vec<String>) -> Result<()> {
    use stock_analysis::lhb_analyzer::LhbDataFetcher;

    let runtime = tokio::runtime::Runtime::new()?;
    match runtime.block_on(async {
        let fetcher = LhbDataFetcher::new()?;
        fetcher.get_today_lhb().await
    }) {
        Ok(mut records) if !records.is_empty() => {
            records.sort_by(|a, b| {
                b.net_amount
                    .partial_cmp(&a.net_amount)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let top_n = 10;
            let before = stock_codes.len();
            for record in records.iter().take(top_n) {
                if record.code.starts_with("92") {
                    continue; // 过滤北交所
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
            info!(
                "🐉 龙虎榜Top{} 新增追加 {} 只（去重后）",
                top_n,
                stock_codes.len() - before
            );
        }
        Ok(_) => info!("📋 今日暂无龙虎榜数据"),
        Err(e) => info!("⚠️ 获取龙虎榜数据失败（不影响正常分析）: {}", e),
    }
    Ok(())
}

fn append_limit_up(stock_codes: &mut Vec<String>) -> HashSet<String> {
    use stock_analysis::market_analyzer::MarketAnalyzer;

    let mut set = HashSet::new();
    let analyzer = match MarketAnalyzer::new(None) {
        Ok(a) => a,
        Err(e) => {
            info!("⚠️ 创建市场分析器失败: {}", e);
            return set;
        }
    };
    match analyzer.get_limit_up_stocks() {
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

/// 板块共振追加：基于东方财富概念板块榜（涨幅 + 主力净流入）与宏观新闻共振，
/// 找出真正在涨、有真金白银且新闻匹配的板块，注入其龙头股。
fn append_sector_resonance(stock_codes: &mut Vec<String>, macro_news: &str) {
    use stock_analysis::market_analyzer::sector_monitor;

    let rank_top = std::env::var("SECTOR_RANK_TOP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20usize);
    let max_sectors = std::env::var("SECTOR_RESONANCE_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5usize);
    let leaders_per_sector = std::env::var("SECTOR_LEADERS_PER")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5usize);

    match sector_monitor::detect_resonance_sectors(
        macro_news,
        rank_top,
        max_sectors,
        leaders_per_sector,
    ) {
        Ok(sectors) if !sectors.is_empty() => {
            let leader_codes = sector_monitor::collect_leader_codes(&sectors);
            let before = stock_codes.len();
            for code in &leader_codes {
                if !stock_codes.contains(code) {
                    stock_codes.push(code.clone());
                }
            }
            info!(
                "🎯 板块共振命中 {} 个板块，候选龙头 {} 只，新增追加 {} 只（去重后）",
                sectors.len(),
                leader_codes.len(),
                stock_codes.len() - before
            );
            for s in &sectors {
                let leaders_desc = s
                    .leaders
                    .iter()
                    .map(|l| format!("{}({} {:+.1}%)", l.name, l.code, l.change_pct))
                    .collect::<Vec<_>>()
                    .join(",");
                info!(
                    "   ↳ {}({}) [{:?}] 涨幅{:.2}% 主力{:.2}亿 加速{:+.2}pp 量比{:.2} 点火涨停{}只 龙头[{}]",
                    s.board.name,
                    s.board.code,
                    s.hit_dims,
                    s.board.change_pct,
                    s.board.main_inflow / 1e8,
                    s.board.inflow_accel(),
                    s.board.vol_ratio,
                    s.ignition.limit_up_count,
                    leaders_desc
                );
            }
        }
        Ok(_) => info!("📋 今日板块共振未命中（涨幅榜与资金榜交集为空）"),
        Err(e) => info!("⚠️ 板块共振检测失败（不影响正常分析）: {:#}", e),
    }
}

fn append_open_positions(stock_codes: &mut Vec<String>) {
    use stock_analysis::database::DatabaseManager;

    let db = match std::panic::catch_unwind(DatabaseManager::get) {
        Ok(db) => db,
        Err(_) => return,
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
        let guard = get_analyzer()
            .lock()
            .expect("AI analyzer mutex 已 poison");
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
