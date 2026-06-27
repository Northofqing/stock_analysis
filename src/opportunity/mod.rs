//! Opportunity Context — 产业链挖掘 + 机会发现。
//!
//! 新闻事件 → 产业链映射 → 持仓影响评估 → 新标的推荐。

pub mod chain_mapper;
pub mod impact;
pub mod discover;

use crate::data_provider::{assess_quality, fetch_financials};
use crate::data_provider::service;
use crate::indicators::{calc_macd, MACD_FAST, MACD_SIGNAL, MACD_SLOW};
use crate::portfolio;
use crate::search_service::SearchResult;

#[derive(Debug, Clone)]
struct PostCloseCandidate {
    base: discover::Candidate,
    profit_score: f64,
    flow_score: f64,
    pattern_score: f64,
    final_score: f64,
    profit_reason: String,
    flow_reason: String,
    flow_source_status: String,
    pattern_reason: String,
    breakout_reason: String,
    breakout_confidence: u8,
    stop_line: Option<f64>,
    quality_note: String,
}

fn classify_flow_error(err: Option<&str>) -> &'static str {
    let Some(e) = err else {
        return "OK";
    };
    if e.contains("Sina") {
        "Sina"
    } else if e.contains("非JSON回包") {
        "非JSON"
    } else if e.contains("状态码") {
        "HTTP"
    } else if e.contains("JSON解析失败") || e.contains("无 klines") {
        "解析"
    } else {
        "其他"
    }
}

fn sma_last(values: &[f64], period: usize) -> Option<f64> {
    if values.len() < period {
        return None;
    }
    let tail = &values[values.len() - period..];
    Some(tail.iter().sum::<f64>() / period as f64)
}

/// 经典「空中加油」三步法评分（盘后和竞价重推共用）：
/// 1) 启动日：大阳/跳空 + 放量
/// 2) 整盘日：不破关键位 + 缩量整固
/// 3) 技术共振：MA 多头 + MACD 零轴上方
fn score_air_refuel_pattern(
    kline: &[crate::data_provider::KlineData],
) -> (f64, String, Option<f64>) {
    if kline.len() < 6 {
        return (0.0, "节奏结构数据不足（<6日），按0分处理".to_string(), None);
    }

    // 以最近三个交易日建模：prev2=启动日候选, prev1=整盘日候选, last=最新收盘
    let prev2 = &kline[kline.len() - 3];
    let prev1 = &kline[kline.len() - 2];
    let last = &kline[kline.len() - 1];
    let prev3 = &kline[kline.len() - 4];

    let mut score: f64 = 0.0;

    // Step 1: 启动日（大阳/跳空 + 放量）
    let launch_big = prev2.pct_chg >= 7.0;
    let launch_gap = prev2.open > prev3.close * 1.01;
    let launch_vol_burst = prev3.volume > 0.0 && prev2.volume >= prev3.volume * 2.0;
    let recent20_max_vol = kline[kline.len().saturating_sub(22)..kline.len() - 2]
        .iter()
        .fold(0.0_f64, |acc, d| acc.max(d.volume));
    let launch_phase_vol_high = prev2.volume >= recent20_max_vol;

    if launch_big {
        score += 2.0;
    }
    if launch_gap {
        score += 1.5;
    }
    if launch_vol_burst || launch_phase_vol_high {
        score += 1.5;
    }

    // Step 2: 整盘日（不补缺口/不破启动日半分位 + 缩量 + K线温和）
    let body_mid = (prev2.open + prev2.close) / 2.0;
    let no_gap_fill = if launch_gap {
        prev1.low > prev3.close
    } else {
        true
    };
    let hold_half = prev1.low >= body_mid;
    let vol_shrink = prev2.volume > 0.0 && prev1.volume <= prev2.volume * 0.65;
    let small_body = prev1.open > 0.0 && ((prev1.close - prev1.open).abs() / prev1.open) <= 0.025;
    let lower_shadow_ok = prev1.low < prev1.close.min(prev1.open);

    if no_gap_fill && hold_half {
        score += 2.5;
    } else if hold_half {
        score += 1.0;
    } else {
        score -= 2.0;
    }
    if vol_shrink {
        score += 2.0;
    } else {
        score -= 1.0;
    }
    if small_body || lower_shadow_ok {
        score += 1.0;
    }

    // Step 3: 均线 + MACD 共振
    let closes: Vec<f64> = kline.iter().map(|d| d.close).collect();
    let ma5 = sma_last(&closes, 5);
    let ma10 = sma_last(&closes, 10);
    let ma20 = sma_last(&closes, 20);
    if let (Some(ma5), Some(ma10), Some(ma20)) = (ma5, ma10, ma20) {
        if last.close >= ma5 && ma5 >= ma10 && ma10 >= ma20 {
            score += 2.0;
        } else if last.close >= ma5 {
            score += 0.5;
        } else {
            score -= 1.0;
        }
        if prev1.low >= ma5 * 0.985 {
            score += 0.8;
        }
    }

    let macd = calc_macd(&closes, MACD_FAST, MACD_SLOW, MACD_SIGNAL);
    let mut macd_note = "MACD数据不足".to_string();
    if macd.len() >= 2 {
        let m0 = &macd[macd.len() - 2];
        let m1 = &macd[macd.len() - 1];
        let zero_axis_ok = m1.dif > 0.0 && m1.dea > 0.0;
        let hist_reacc = m1.histogram > m0.histogram;
        if zero_axis_ok {
            score += 1.0;
        } else {
            score -= 1.0;
        }
        if hist_reacc {
            score += 1.0;
        }
        macd_note = format!(
            "DIF={:+.3} DEA={:+.3} 柱={:+.3}->{:+.3}",
            m1.dif, m1.dea, m0.histogram, m1.histogram
        );
    }

    let stop_line = if launch_gap {
        Some(prev1.low.max(prev3.close))
    } else {
        Some(prev1.low.max(body_mid))
    };

    let score = score.clamp(-6.0, 12.0);
    let reason = format!(
        "节奏结构{:+.1}：启动日{}{}{}；整盘日{}{}{}；技术共振({})；硬止损参考={}",
        score,
        if launch_big { "涨幅>7% " } else { "涨幅不足 " },
        if launch_gap { "跳空成立 " } else { "无有效跳空 " },
        if launch_vol_burst || launch_phase_vol_high { "放量成立" } else { "放量不足" },
        if no_gap_fill { "缺口未补 " } else { "缺口回补 " },
        if vol_shrink { "缩量 " } else { "未缩量 " },
        if small_body || lower_shadow_ok { "K线温和" } else { "K线偏弱" },
        macd_note,
        stop_line
            .map(|v| format!("¥{:.2}", v))
            .unwrap_or_else(|| "NA".to_string())
    );

    (score, reason, stop_line)
}

fn score_breakout_structure(sig: &crate::breakout::signal::BreakoutSignal) -> (f64, String) {
    use crate::breakout::signal::{BreakoutType, CandleStrength, VolumePattern};

    let mut score = match sig.breakout_type {
        BreakoutType::Launch => 1.8,
        BreakoutType::Uncertain => 0.2,
        BreakoutType::Distribution => -1.8,
    };

    score += match sig.volume_pattern {
        VolumePattern::PostShrinkBurst => 1.4,
        VolumePattern::GentleIncrease => 0.8,
        VolumePattern::SuddenSpike => -0.4,
        VolumePattern::Flat => -1.2,
    };

    score += match sig.candle_strength {
        CandleStrength::Strong => 0.8,
        CandleStrength::Medium => 0.4,
        CandleStrength::Weak => 0.0,
        CandleStrength::Bearish => -0.8,
    };

    score += ((sig.confidence as f64 - 50.0) / 20.0).clamp(-1.5, 1.5);
    let score = score.clamp(-4.0, 4.0);

    let reason = format!(
        "{} / {} / {} / 置信{}%",
        sig.breakout_type.label(),
        sig.volume_pattern.label(),
        sig.candle_strength.label(),
        sig.confidence,
    );
    (score, reason)
}

fn score_profit_elasticity(fin: &crate::data_provider::Financials) -> (f64, String, String) {
    if !fin.any() {
        return (0.0, "财务数据不足，利润弹性按0分处理".to_string(), "盈利质量未知（数据不足）".to_string());
    }

    let mut score: f64 = 0.0;

    if let (Some(np), Some(rev)) = (fin.net_profit_yoy, fin.revenue_yoy) {
        if np >= 15.0 {
            score += 2.0;
        }
        if rev >= 10.0 {
            score += 1.5;
        }
        if np > rev {
            score += 1.5;
        }
    }

    if let Some(gm) = fin.gross_margin {
        if gm >= 25.0 {
            score += 1.5;
        } else if gm >= 15.0 {
            score += 0.8;
        }
    }

    if let Some(roe) = fin.roe {
        if roe >= 15.0 {
            score += 1.5;
        } else if roe >= 10.0 {
            score += 0.8;
        }
    }

    let quality_note = if let Some(q) = assess_quality(&fin.history) {
        if q.risk_score >= 60 {
            score -= 3.0;
        } else if q.risk_score >= 30 {
            score -= 1.5;
        } else {
            score += 0.5;
        }
        format!("{}（风险分{}）", q.level, q.risk_score)
    } else {
        "盈利质量未知".to_string()
    };

    let score = score.clamp(-4.0, 8.0);
    let reason = format!(
        "利润弹性{:+.1}：净利YoY={} 营收YoY={} 毛利率={} ROE={}，{}",
        score,
        fin.net_profit_yoy.map(|v| format!("{:+.1}%", v)).unwrap_or_else(|| "NA".to_string()),
        fin.revenue_yoy.map(|v| format!("{:+.1}%", v)).unwrap_or_else(|| "NA".to_string()),
        fin.gross_margin.map(|v| format!("{:.1}%", v)).unwrap_or_else(|| "NA".to_string()),
        fin.roe.map(|v| format!("{:.1}%", v)).unwrap_or_else(|| "NA".to_string()),
        quality_note,
    );
    (score, reason, quality_note)
}

fn score_capital_consensus(
    flow: &crate::data_provider::MoneyFlowSummary,
    fetch_error: Option<&str>,
) -> (f64, String) {
    if flow.is_empty() {
        let why = fetch_error
            .map(|s| {
                let compact = s.replace('\n', " ");
                if compact.chars().count() > 80 {
                    format!("{}...", compact.chars().take(80).collect::<String>())
                } else {
                    compact
                }
            })
            .unwrap_or_else(|| "未知原因".to_string());
        return (0.0, format!("资金数据不可用（{}），资金共识按0分处理", why));
    }

    let latest_main_yi = flow.latest().map(|d| d.main_net / 1e8).unwrap_or(0.0);
    let sum5_yi = flow.recent_main_sum(5) / 1e8;
    let ewma_yi = flow.ewma_main_net_yi().unwrap_or(0.0);

    let mut score: f64 = 0.0;
    if latest_main_yi > 0.0 {
        score += 1.5;
    } else {
        score -= 1.0;
    }
    if sum5_yi > 0.0 {
        score += 2.0;
    } else {
        score -= 1.5;
    }
    if ewma_yi > 0.0 {
        score += 1.0;
    } else {
        score -= 0.8;
    }
    if flow.is_one_day_bounce() {
        score -= 1.0;
    }

    let score = score.clamp(-4.0, 5.0);
    let reason = format!(
        "资金共识代理{:+.1}：当日主力{:+.2}亿，近5日累计{:+.2}亿，5日EWMA{:+.2}亿{}",
        score,
        latest_main_yi,
        sum5_yi,
        ewma_yi,
        if flow.is_one_day_bounce() { "，存在单日反弹未逆转迹象" } else { "" },
    );
    (score, reason)
}

fn normalize_text(s: &str) -> String {
    s.to_lowercase().replace(' ', "")
}

fn truncate_for_prompt(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    format!("{}...", s.chars().take(max_chars).collect::<String>())
}

fn build_chain_event_evidence(
    hits: &[chain_mapper::ChainHit],
    titles: &[String],
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for hit in hits {
        let mut matched: Vec<String> = Vec::new();
        for t in titles {
            if text_matches_hit(t, hit) {
                matched.push(truncate_for_prompt(t, 30));
            }
            if matched.len() >= 2 {
                break;
            }
        }
        let evidence = if matched.is_empty() {
            format!(
                "关键词触发：{}（未检索到可展示标题片段）",
                hit.keywords.join("、")
            )
        } else {
            format!(
                "关键词触发：{}；事件样本：{}",
                hit.keywords.join("、"),
                matched.join(" | ")
            )
        };
        out.insert(hit.chain.clone(), evidence);
    }
    out
}

fn text_matches_hit(text: &str, hit: &chain_mapper::ChainHit) -> bool {
    let t = normalize_text(text);
    if t.is_empty() {
        return false;
    }
    if !hit.board_keyword.is_empty() {
        let b = normalize_text(&hit.board_keyword);
        if !b.is_empty() && (t.contains(&b) || b.contains(&t)) {
            return true;
        }
    }
    let chain = normalize_text(&hit.chain);
    if !chain.is_empty() && (t.contains(&chain) || chain.contains(&t)) {
        return true;
    }
    hit.keywords.iter().any(|kw| {
        let k = normalize_text(kw);
        k.len() >= 2 && (t.contains(&k) || k.contains(&t))
    })
}

fn score_hit_confidence(
    hit: &chain_mapper::ChainHit,
    flash_titles: &[String],
    web_results: &[SearchResult],
) -> (u8, bool, String) {
    let mut score: i32 = match hit.source {
        chain_mapper::ChainSource::Rule => 28,
        chain_mapper::ChainSource::Ai => 16,
        chain_mapper::ChainSource::AiDegraded => 8,
    };

    if !hit.board_keyword.trim().is_empty() {
        score += 5;
    }

    let flash_hits = flash_titles
        .iter()
        .filter(|t| text_matches_hit(t, hit))
        .count() as i32;
    score += (flash_hits * 6).min(24);

    let mut web_hits = 0_i32;
    let mut web_importance = 0_i32;
    for r in web_results {
        let text = format!("{} {}", r.title, r.snippet);
        if text_matches_hit(&text, hit) {
            web_hits += 1;
            web_importance += r.importance as i32;
        }
    }
    score += (web_hits * 5).min(20);
    score += (web_importance / 4).min(12);

    let cross = flash_hits > 0 && web_hits > 0;
    if cross {
        score += 16;
    }

    let score = score.clamp(0, 100) as u8;
    let reason = format!(
        "score={} src={:?} flash_hits={} web_hits={} cross={}",
        score, hit.source, flash_hits, web_hits, cross
    );
    (score, cross, reason)
}

fn gate_hits_by_confidence(
    hits: Vec<chain_mapper::ChainHit>,
    flash_titles: &[String],
    web_results: &[SearchResult],
) -> (Vec<chain_mapper::ChainHit>, Vec<String>) {
    let cfg = crate::config::get_monitor_config();
    let threshold = cfg.opportunity_min_confidence;
    let require_cross = cfg.opportunity_require_cross_source;

    let mut kept = Vec::new();
    let mut dropped = Vec::new();

    for hit in hits {
        let (score, cross, reason) = score_hit_confidence(&hit, flash_titles, web_results);
        let pass = score >= threshold && (!require_cross || cross);
        if pass {
            kept.push(hit);
        } else {
            dropped.push(format!("{}: {}", hit.chain, reason));
        }
    }

    (kept, dropped)
}

/// 二次门控：仅保留量能/趋势向上的候选。
///
/// 通过条件：
/// - 趋势向上：BreakoutType::Launch 且 confidence >= 45
/// - 变盘向上：BreakoutType::Uncertain 且 confidence >= 50，且出现地量后放量/温和放量，
///   同时 K 线不为阴线。
fn is_uptrend_or_upturn(sig: &crate::breakout::signal::BreakoutSignal) -> bool {
    use crate::breakout::signal::{BreakoutType, CandleStrength, VolumePattern};

    if sig.breakout_type == BreakoutType::Launch && sig.confidence >= 45 {
        return true;
    }

    sig.breakout_type == BreakoutType::Uncertain
        && sig.confidence >= 50
        && matches!(
            sig.volume_pattern,
            VolumePattern::PostShrinkBurst | VolumePattern::GentleIncrease
        )
        && !matches!(sig.candle_strength, CandleStrength::Bearish)
}

fn breakout_gate_candidates(
    candidates: Vec<discover::Candidate>,
) -> Vec<(discover::Candidate, crate::breakout::signal::BreakoutSignal)> {
    let fetcher = match crate::data_provider::DataFetcherManager::new() {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[Opportunity] 初始化数据抓取器失败，跳过趋势二次门控: {:#}", e);
            return Vec::new();
        }
    };

    let mut passed = Vec::new();
    for c in candidates {
        let Ok((kline, _)) = fetcher.get_daily_data(&c.code, 60) else {
            continue;
        };
        let sig = crate::breakout::engine::analyze_postmarket(&c.code, &c.name, &kline);
        if is_uptrend_or_upturn(&sig) {
            passed.push((c, sig));
        }
    }
    passed
}

/// 产业链扫描结果：产业链正文与持仓影响分离，便于分开推送。
pub struct OpportunityScan {
    /// 产业链扫描正文（含受益标的、值得关注、卖飞复盘）。
    pub chain_text: String,
    /// 持仓影响正文。空 = 无持仓影响，不推送。
    pub impact_text: String,
}

/// 运行一次产业链扫描，返回「产业链」与「持仓影响」分离的结果
pub async fn run_opportunity_scan() -> OpportunityScan {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("📡 产业链扫描（{}）", chrono::Local::now().format("%H:%M")));
    lines.push("━━━━━━━━━━━━━━━━━━━━━━━━".to_string());

    // 1. 获取最新快讯标题
    let svc = crate::search_service::get_search_service();
    let flash_titles = svc.fetch_flash_titles(30).await;
    let flash_n = flash_titles.len();
    let mut titles = flash_titles.clone();

    // 1b. (T6a) Web 搜索补充今日重大新闻维度（失败容忍，不阻断；数据红线 2.1 不伪造）
    let mut web_results: Vec<SearchResult> = Vec::new();
    if svc.is_available() {
        let web = svc.search_topic("今日 A股 重大新闻 政策 产业", 8).await;
        for r in web {
            // 仅纳入有一定重要性的，避免噪声
            if r.importance >= 5 && !r.title.trim().is_empty() {
                titles.push(r.title.clone());
                web_results.push(r);
            }
        }
    }
    let web_n = web_results.len();

    if titles.is_empty() {
        log::info!("[Opportunity] 采集 0 条新闻 → 跳过本轮");
        lines.push("暂无最新快讯".to_string());
        return OpportunityScan { chain_text: lines.join("\n"), impact_text: String::new() };
    }

    // 2. 产业链映射（规则优先，未命中则 AI 兜底）
    let mut hits = chain_mapper::map_news_to_chains_ai(&titles).await;
    if hits.is_empty() {
        log::info!("[Opportunity] 采集 {} 条新闻(快讯{}/Web{}) → 命中 0 条产业链（含AI兜底）", titles.len(), flash_n, web_n);
        lines.push("当前快讯未命中已知产业链".to_string());
        return OpportunityScan { chain_text: lines.join("\n"), impact_text: String::new() };
    }
    let rule_n = hits.iter().filter(|h| h.source == chain_mapper::ChainSource::Rule).count();
    let ai_n = hits.iter().filter(|h| h.source == chain_mapper::ChainSource::Ai).count();

    // 3. 动态解析标的（sector_monitor 内部用 reqwest::blocking，走 spawn_blocking）
    let mut hits = tokio::task::spawn_blocking(move || {
        chain_mapper::resolve_stocks(&mut hits);
        hits
    }).await.unwrap_or_default();
    hits.retain(|h| !h.stocks.is_empty());
    let (hits, dropped) = gate_hits_by_confidence(hits, &flash_titles, &web_results);
    if hits.is_empty() {
        log::warn!(
            "[Opportunity] 采集 {} 条新闻(快讯{}/Web{}) → 候选被置信度门控全部过滤: {}",
            titles.len(), flash_n, web_n, dropped.join(" | ")
        );
        return OpportunityScan {
            chain_text: "当前产业链信号可信度不足（已降级观察）".to_string(),
            impact_text: String::new(),
        };
    }

    // 4. 持仓影响评估
    let holdings = portfolio::get_positions().unwrap_or_default();
    let our_codes: Vec<String> = holdings.iter().map(|p| p.code.clone()).collect();
    let impacts = impact::assess_impact(&hits, &holdings);

    // 5. 输出
    for hit in &hits {
        lines.push(String::new());
        lines.push(format!("🔥 {}：{}", hit.chain, hit.logic));
        lines.push(format!("  关键词：{}", hit.keywords.join("、")));
        let stock_list: Vec<String> = hit.stocks.iter().take(5)
            .map(|s| format!("{}({})", s.name, s.code)).collect();
        lines.push(format!("  受益标的：{}", stock_list.join(" ")));
    }

    // 持仓影响（独立成段，分开推送）
    let mut impact_lines: Vec<String> = Vec::new();
    if !impacts.is_empty() {
        impact_lines.push(format!("📌 持仓影响（{}）", chrono::Local::now().format("%H:%M")));
        impact_lines.push("━━━━━━━━━━━━━━━━━━━━━━━━".to_string());
        for imp in &impacts {
            impact_lines.push(format!("  {} {}({}) — {}: {}",
                imp.direction.emoji(), imp.name, imp.code,
                imp.direction.label(), imp.reason));
        }
    }

    // 新标的推荐（先产业链，再量能/趋势二次门控）
    let candidates = discover::discover(&hits, &our_codes, 6);
    let gated = tokio::task::spawn_blocking(move || breakout_gate_candidates(candidates))
        .await
        .unwrap_or_default();
    if !gated.is_empty() {
        lines.push(String::new());
        lines.push("🎯 值得关注：".to_string());
        for (c, sig) in gated.iter().take(3) {
            let note = if c.price_note.is_empty() { String::new() } else { format!(" [{}]", c.price_note) };
            lines.push(format!(
                "  {}({}) — {}：{}{} | 量能/趋势确认: {} {} 置信{} [{}]",
                c.name,
                c.code,
                c.chain,
                c.logic,
                note,
                sig.breakout_type.label(),
                sig.volume_pattern.label(),
                sig.confidence,
                sig.description,
            ));
            // 影子盘留痕（合规可追溯，AGENTS.md 2.7）：机会以"看多"记入 prediction_tracker
            crate::monitor::prediction::save_prediction(
                Some(&c.chain), Some(&c.code), "看多", 60.0, Some(&c.logic),
            );
        }
    } else {
        lines.push(String::new());
        lines.push("🎯 值得关注：暂无通过量能/趋势确认的候选（等待趋势向上或变盘向上）".to_string());
    }

    // (T6b) 卖飞复盘：已平仓标的今日又走强且出现在机会产业链中 → 提醒“可能卖飞”
    let sold_review = review_sold_too_early(&hits, &our_codes);
    if !sold_review.is_empty() {
        lines.push(String::new());
        lines.push("⏪ 卖飞复盘（已卖出但产业链再起）：".to_string());
        for r in &sold_review {
            lines.push(format!("  {}", r));
        }
    }

    log::info!(
        "[Opportunity] 采集 {} 条新闻(快讯{}/Web{}) → 命中产业链 {} 条(规则{}/AI{}) → 趋势二次门控后推送 {} 个机会",
        titles.len(), flash_n, web_n, hits.len(), rule_n, ai_n, gated.len().min(3)
    );

    OpportunityScan {
        chain_text: lines.join("\n"),
        impact_text: if impact_lines.is_empty() { String::new() } else { impact_lines.join("\n") },
    }
}

/// 优选候选（与盘中信号分维度）：
/// - 支持盘后与集合竞价重推，只输出最多 Top N
/// - 强制输出入选原因，便于复盘与执行前二次确认
pub async fn run_post_close_candidates(top_n: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("🎯 优选候选（{}）", chrono::Local::now().format("%H:%M")));
    lines.push("━━━━━━━━━━━━━━━━━━━━━━━━".to_string());

    // 1. 获取新闻
    let svc = crate::search_service::get_search_service();
    let flash_titles = svc.fetch_flash_titles(40).await;
    let mut titles = flash_titles.clone();
    let mut web_results: Vec<SearchResult> = Vec::new();
    if svc.is_available() {
        let web = svc.search_topic("今日 A股 重大新闻 政策 产业", 10).await;
        for r in web {
            if r.importance >= 5 && !r.title.trim().is_empty() {
                titles.push(r.title.clone());
                web_results.push(r);
            }
        }
    }

    if titles.is_empty() {
        return "🎯 优选候选：暂无可用新闻，跳过本轮".to_string();
    }

    // 2. 产业链映射 + 标的解析
    let mut hits = chain_mapper::map_news_to_chains_ai(&titles).await;
    if hits.is_empty() {
        return "🎯 优选候选：未命中产业链，不输出候选".to_string();
    }
    let mut hits = tokio::task::spawn_blocking(move || {
        chain_mapper::resolve_stocks(&mut hits);
        hits
    })
    .await
    .unwrap_or_default();
    hits.retain(|h| !h.stocks.is_empty());
    let (hits, _dropped) = gate_hits_by_confidence(hits, &flash_titles, &web_results);
    if hits.is_empty() {
        return "🎯 优选候选：产业链信号可信度不足（已降级观察）".to_string();
    }
    let event_evidence = build_chain_event_evidence(&hits, &titles);

    // 3. 排除已持仓，先做链路/位置初选
    let holdings = portfolio::get_positions().unwrap_or_default();
    let our_codes: Vec<String> = holdings.iter().map(|p| p.code.clone()).collect();
    let raw = discover::discover(&hits, &our_codes, top_n * 3);

    // 4. 候选二次评分：利润弹性 + 资金共识代理 + 空中加油形态（盘后与竞价重推共用）
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(5))
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
             AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        )
        .build()
        .unwrap_or_default();

    // 量能硬门槛：量能不足直接丢弃，不进入推送。
    const POST_CLOSE_MIN_VOL_RATIO: f64 = 1.20;
    let mut rescored: Vec<PostCloseCandidate> = Vec::new();
    let mut flow_ok_count = 0usize;
    let mut flow_fail_count = 0usize;
    let mut flow_fail_non_json = 0usize;
    let mut flow_fail_http = 0usize;
    let mut flow_fail_parse = 0usize;
    let mut flow_fail_other = 0usize;
    for c in raw {
        let fin = fetch_financials(&client, &c.code);
        let (flow, flow_fetch_error) =
            match crate::data_provider::money_flow::fetch_flow_history_async(&client, &c.code, 8)
                .await
            {
                Ok(s) => {
                    flow_ok_count += 1;
                    (s, None)
                }
                Err(e) => {
                    log::warn!("[资金流] {} 抓取失败: {}", c.code, e);
                    flow_fail_count += 1;
                    let msg = e.to_string();
                    if msg.contains("非JSON回包") {
                        flow_fail_non_json += 1;
                    } else if msg.contains("状态码") {
                        flow_fail_http += 1;
                    } else if msg.contains("JSON解析失败") || msg.contains("无 klines") {
                        flow_fail_parse += 1;
                    } else {
                        flow_fail_other += 1;
                    }
                    (crate::data_provider::MoneyFlowSummary::default(), Some(e.to_string()))
                }
            };
        let kline = service::service().get_kline(&c.code, 80).await.ok();

        let (profit_score, profit_reason, quality_note) = score_profit_elasticity(&fin);
        let (flow_score, flow_reason) =
            score_capital_consensus(&flow, flow_fetch_error.as_deref());
        let flow_source_status = classify_flow_error(flow_fetch_error.as_deref()).to_string();
        let (pattern_score, pattern_reason, stop_line) = kline
            .as_deref()
            .map(|k| {
                let (rhythm_score, rhythm_reason, stop_line) = score_air_refuel_pattern(k);
                let sig = crate::breakout::engine::analyze_postmarket(&c.code, &c.name, k);
                let (breakout_score, breakout_reason) = score_breakout_structure(&sig);
                let total = (rhythm_score + breakout_score).clamp(-8.0, 14.0);
                (
                    total,
                    format!("{}；放量结构{:+.1}：{}", rhythm_reason, breakout_score, breakout_reason),
                    stop_line,
                )
            })
            .unwrap_or((0.0, "K线数据抓取失败，趋势结构按0分处理".to_string(), None));
        let (breakout_reason, breakout_confidence, volume_ok) = kline
            .as_deref()
            .map(|k| {
                let sig = crate::breakout::engine::analyze_postmarket(&c.code, &c.name, k);
                let vol_ratio = sig.vol_vs_20d_avg.unwrap_or(0.0);
                (
                    format!(
                        "{} 置信{}% 量能{:.2}x [{}]",
                        sig.breakout_type.label(),
                        sig.confidence,
                        vol_ratio,
                        sig.description
                    ),
                    sig.confidence,
                    vol_ratio >= POST_CLOSE_MIN_VOL_RATIO,
                )
            })
            .unwrap_or(("K线数据抓取失败，放量分析不可用".to_string(), 0, false));
        if !volume_ok {
            continue;
        }
        let final_score = c.score + profit_score + flow_score + pattern_score;

        rescored.push(PostCloseCandidate {
            base: c,
            profit_score,
            flow_score,
            pattern_score,
            final_score,
            profit_reason,
            flow_reason,
            flow_source_status,
            pattern_reason,
            breakout_reason,
            breakout_confidence,
            stop_line,
            quality_note,
        });
    }

    rescored.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // 盘后更强调"优中选优"：设置最低分，分不够宁缺毋滥。
    const POST_CLOSE_MIN_SCORE: f64 = 20.0;
    let picked: Vec<PostCloseCandidate> = rescored
        .into_iter()
        .filter(|c| c.final_score >= POST_CLOSE_MIN_SCORE)
        .take(top_n)
        .collect();

    if picked.is_empty() {
        return format!(
            "🎯 优选候选：本轮无满足阈值的标的（总分阈值 >= {:.1}，量能阈值 >= {:.2}x）",
            POST_CLOSE_MIN_SCORE,
            POST_CLOSE_MIN_VOL_RATIO
        );
    }

    lines.push(format!(
        "共 {} 只入选（总分阈值 >= {:.1}，量能阈值 >= {:.2}x，按2.1评分排序）",
        picked.len(),
        POST_CLOSE_MIN_SCORE,
        POST_CLOSE_MIN_VOL_RATIO
    ));
    lines.push(format!(
        "资金共识数据可用率: {}/{}（失败{}：非JSON={} HTTP={} 解析={} 其他={}）",
        flow_ok_count,
        flow_ok_count + flow_fail_count,
        flow_fail_count,
        flow_fail_non_json,
        flow_fail_http,
        flow_fail_parse,
        flow_fail_other,
    ));
    lines.push(String::new());

    for (idx, c) in picked.iter().enumerate() {
        lines.push(format!("{}. {}({})", idx + 1, c.base.name, c.base.code));
        lines.push(format!(
            "   总分: {:.1} = 链路{:.1} + 利润{:+.1} + 资金{:+.1} + 结构{:+.1}",
            c.final_score, c.base.score, c.profit_score, c.flow_score, c.pattern_score
        ));
        let event_text = event_evidence
            .get(&c.base.chain)
            .cloned()
            .unwrap_or_else(|| "事件证据暂缺（请查看产业链扫描原文）".to_string());
        lines.push(format!("   ① 事件触发: {}", event_text));
        lines.push(format!(
            "   ② 产业链位置: {} | {} | 个股态势: {}",
            c.base.chain,
            c.base.logic,
            if c.base.price_note.is_empty() {
                "位置中性"
            } else {
                c.base.price_note.as_str()
            }
        ));
        lines.push(format!("   ③ 利润弹性: {}", c.profit_reason));
        lines.push(format!(
            "   ④ 机构/资金共识(代理): {} [资金源:{}]",
            c.flow_reason,
            c.flow_source_status
        ));
        lines.push(format!("   ⑤ 股价位置: {}", c.base.reason_summary));
        lines.push(format!("   ⑥ 放量分析: {}", c.breakout_reason));
        lines.push(format!("   ⑦ 趋势结构(多模型): {}", c.pattern_reason));
        if c.base.price_note.is_empty() {
            lines.push(format!(
                "   ⑧ 风险与执行: {}；放量置信={}%；止损线={}；暂未触发明显追高信号，次日竞价与开盘15分钟需确认弱转强",
                c.quality_note,
                c.breakout_confidence,
                c.stop_line
                    .map(|v| format!("¥{:.2}", v))
                    .unwrap_or_else(|| "NA".to_string())
            ));
        } else {
            lines.push(format!(
                "   ⑧ 风险与执行: {}；{}；放量置信={}%；止损线={}；次日竞价与开盘15分钟需确认弱转强",
                c.quality_note,
                c.base.price_note,
                c.breakout_confidence,
                c.stop_line
                    .map(|v| format!("¥{:.2}", v))
                    .unwrap_or_else(|| "NA".to_string())
            ));
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

/// 卖飞复盘：从近期卖出交易中找出"今日又出现在机会产业链受益标的中"的，
/// 提示可能卖出过早。仅用真实卖出记录（Trade，Sell 方向），不编造。
fn review_sold_too_early(hits: &[chain_mapper::ChainHit], owned_codes: &[String]) -> Vec<String> {
    use crate::portfolio::TradeDirection;
    let owned: std::collections::HashSet<&str> = owned_codes.iter().map(|c| c.as_str()).collect();
    // 近 20 个自然日的卖出记录
    let trades = match portfolio::get_trade_history(20) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for t in &trades {
        if t.direction != TradeDirection::Sell { continue; }
        if owned.contains(t.code.as_str()) { continue; } // 已重新持仓不算卖飞
        for hit in hits {
            if hit.stocks.iter().any(|s| s.code == t.code) {
                if !seen.insert(t.code.clone()) { continue; }
                out.push(format!("{}({}) 已卖出，今在[{}]再起：{}", t.name, t.code, hit.chain, hit.logic));
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::score_air_refuel_pattern;
    use crate::data_provider::KlineData;

    fn kd(open: f64, high: f64, low: f64, close: f64, vol: f64, pct: f64) -> KlineData {
        KlineData {
            date: chrono::NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            open,
            high,
            low,
            close,
            volume: vol,
            amount: vol * close,
            pct_chg: pct,
            intraday_price: None,
            settled: true,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
        }
    }

    #[test]
    fn test_air_refuel_scores_positive_on_clean_pattern() {
        let k = vec![
            kd(9.8, 10.0, 9.7, 9.9, 1_000.0, 0.5),
            kd(9.9, 10.1, 9.8, 10.0, 1_100.0, 1.0),
            kd(10.0, 10.2, 9.9, 10.1, 1_050.0, 1.0),
            kd(10.5, 11.2, 10.45, 11.0, 2_800.0, 8.9),
            kd(11.0, 11.1, 10.8, 10.95, 1_400.0, -0.5),
            kd(11.0, 11.3, 10.95, 11.2, 2_000.0, 2.3),
        ];
        let (score, reason, stop_line) = score_air_refuel_pattern(&k);
        assert!(score > 2.0, "{}", reason);
        assert!(stop_line.is_some());
    }

    #[test]
    fn test_air_refuel_penalized_when_consolidation_breaks() {
        let k = vec![
            kd(9.8, 10.0, 9.7, 9.9, 1_000.0, 0.5),
            kd(9.9, 10.1, 9.8, 10.0, 1_100.0, 1.0),
            kd(10.0, 10.2, 9.9, 10.1, 1_050.0, 1.0),
            kd(10.5, 11.2, 10.45, 11.0, 2_800.0, 8.9),
            kd(10.9, 11.0, 10.1, 10.2, 2_900.0, -7.3),
            kd(10.2, 10.3, 10.0, 10.1, 2_200.0, -1.0),
        ];
        let (score, _, _) = score_air_refuel_pattern(&k);
        assert!(score < 5.0, "consolidation breakout should be penalized, got {score}");
    }
}
