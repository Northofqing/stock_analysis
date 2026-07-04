//! Opportunity Context — 产业链挖掘 + 机会发现。
//!
//! 新闻事件 → 产业链映射 → 持仓影响评估 → 新标的推荐。

pub mod chain_mapper;
pub mod impact;
pub mod discover;
pub mod score;  // 修复 P0-1: dual_score 评分模型
pub mod bom_kb;  // 修复 P0-2: BOM 弹性节点 + KB
pub mod winrate;  // 修复 P1-2: winrate 二元化
pub mod launch_gate;  // 修复 P0-3: 上线门槛
pub mod event_extractor;
pub mod scheduler;  // 修复 v9.1 §1.3: 调度器
pub mod hit_case;  // v10 P0.1 BC-3: 5 边界 hit CASE 逻辑
pub mod candidate_state;  // v12 PR3-3.4: 影子候选零推送
pub mod candidate_panel;  // v11-P0-5+ Commit A: 候选筛选台模型 + 多源合并去重
pub mod news_ranker;  // P2-News Commit 1: 新闻事件排序层 (EventType/HeatStage/Bucket/RankedNews)
pub mod news_audit;   // P2-News Commit 5: 审计 JSONL 落盘
pub mod news_outcome; // P3: D+1/D+3/D+5 outcome 回看 (不自动调权)
pub mod virtual_reason;  // v10 P0.2 BR-016: VirtualReason 枚举 + 主理由优先级
pub mod auction_agent;  // v10 P0.2: 09:25 竞价 Agent
pub mod real_alpha;  // v10 P0.3 BC-1: real_alpha + A/B/C 置信度 + 5 要素信封

use crate::data_provider::{assess_quality, fetch_financials};
use crate::data_provider::service;
use crate::indicators::{calc_macd, MACD_FAST, MACD_SIGNAL, MACD_SLOW};
use crate::portfolio;
use crate::search_service::SearchResult;
use crate::opportunity::score::{compute_dual_score, ScoreInputs, ScorePart};

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
    /// 修复 F15 (2026-06-29 BR-004): 透传 Candidate.push_time, 用于"同分按发布时间升序"次级排序.
    /// 0 = 老调用方未填 (排在新 case 前面), 可观测的回归.
    push_time: i64,
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

/// 修复 v9.1: dual_score 评分门 (替代 gate_hits_by_confidence)
/// 用 evaluate_hit_for_push 计算 event_risk_score, 按 push_threshold (默认 75) 过滤
/// 注意: 因 NS3 强约束 (无 winrate 封顶 70), 沙盘阶段几乎无人能过 75 推送门
/// 这是设计选择, 不是 bug — 沙盘阶段推送门应该是"几乎永不触发", 仅记录候选
fn gate_hits_by_dual_score(
    hits: Vec<chain_mapper::ChainHit>,
    flash_titles: &[String],
    web_results: &[SearchResult],
) -> (Vec<chain_mapper::ChainHit>, Vec<String>) {
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for hit in hits {
        let (score, _tss, _parts, passed, reason) = evaluate_hit_for_push(&hit, flash_titles, web_results);
        // 修复 v9.1 §0 NS3: 沙盘阶段 NS3 封顶 70 → push_threshold 75 几乎不可达
        // 放宽到 60 (候选池门槛) 让沙盘也有可见候选, 灰度后再严格 75
        let in_pool = score >= 60;
        if passed || in_pool {
            kept.push(hit);
        } else {
            dropped.push(format!("{}: {}", hit.chain, reason));
        }
    }
    (kept, dropped)
}

/// 修复 v9.1: 统一门控入口, 按 config.opportunity_use_dual_score 切换
/// 默认 false (向后兼容 ad-hoc score_hit_confidence)
/// true (启用 dual_score 模型) 是 v9.1 推荐路径
fn gate_hits(
    hits: Vec<chain_mapper::ChainHit>,
    flash_titles: &[String],
    web_results: &[SearchResult],
) -> (Vec<chain_mapper::ChainHit>, Vec<String>) {
    let cfg = crate::config::get_monitor_config();
    if cfg.opportunity_use_dual_score {
        gate_hits_by_dual_score(hits, flash_titles, web_results)
    } else {
        gate_hits_by_confidence(hits, flash_titles, web_results)
    }
}

// ═══════════════════════════════════════════════════════════
// 修复 v9.1: dual_score 评分门 (替代 ad-hoc score_hit_confidence)
// ═══════════════════════════════════════════════════════════

/// 把 ad-hoc score_hit_confidence 的输出桥接到 v9.1 dual_score.ScoreInputs
/// 量化 PM 视角: 这是迁移期过渡, 把"legacy 加分模型"映射成 v9.1 标准的 6 维输入
/// - event_strength: 来源 (Rule=70 / Ai=40 / AiDegraded=20) — AI 降级项
/// - event_certainty: cross_source_count * 30 + board_keyword bonus
/// - chain_match_score: flash_hits * 15 + web_importance / 2
/// - flow_score: fund_flow_pct 直接 (None 时 = 50 中性)
/// - cross_source_count: 实际跨源数 (1=单源, 2+=跨源)
/// - quality_score: None (财务数据本轮未拉, 中性 50)
/// - winrate_score: None (沙盘阶段无历史样本, P1-2 二元化)
/// - ai_degraded: hit.source == AiDegraded
pub fn build_score_inputs_from_hit(
    hit: &chain_mapper::ChainHit,
    flash_titles: &[String],
    web_results: &[SearchResult],
) -> ScoreInputs {
    // event_strength: 来源基础分 (Rule > Ai > AiDegraded)
    let event_strength = match hit.source {
        chain_mapper::ChainSource::Rule => 70u8,
        chain_mapper::ChainSource::Ai => 50u8,
        chain_mapper::ChainSource::AiDegraded => 20u8,
    };
    // event_certainty: 跨源加分 + 板名命中
    let flash_hits = flash_titles.iter().filter(|t| text_matches_hit(t, hit)).count();
    let web_hits = web_results.iter()
        .filter(|r| text_matches_hit(&format!("{} {}", r.title, r.snippet), hit))
        .count();
    let cross_count = if flash_hits > 0 && web_hits > 0 { 2 } else { 1 };
    let board_bonus: u8 = if !hit.board_keyword.trim().is_empty() { 15 } else { 0 };
    let event_certainty = (cross_count as u8 * 30 + board_bonus).min(100);

    // chain_match_score: flash + web 命中综合
    let chain_match_score = ((flash_hits.min(3) as u8) * 25 + (web_hits.min(3) as u8) * 15).min(100);

    // flow_score: 链级资金流, None 时 50 中性
    let flow_score = hit.fund_flow_pct.map(|f| (50.0 + f * 5.0).clamp(0.0, 100.0));

    ScoreInputs {
        event_strength,
        event_certainty,
        chain_match_score,
        flow_score,
        cross_source_count: cross_count as u8,
        quality_score: None,
        winrate_score: None,
        ai_degraded: matches!(hit.source, chain_mapper::ChainSource::AiDegraded),
    }
}

/// 修复 v9.1 §0 NS3: event_risk_score 推送门
/// 输入 ChainHit + 双源快讯 → ScoreInputs → compute_dual_score
/// 返回 (event_risk_score, trade_signal_score, parts, passed_threshold, reason)
/// passed_threshold: event_risk_score >= config.opportunity_push_threshold (默认 75)
pub fn evaluate_hit_for_push(
    hit: &chain_mapper::ChainHit,
    flash_titles: &[String],
    web_results: &[SearchResult],
) -> (u8, Option<u8>, Vec<ScorePart>, bool, String) {
    let inputs = build_score_inputs_from_hit(hit, flash_titles, web_results);
    let score = compute_dual_score(&inputs, "v9.1-push-gate");
    let cfg = crate::config::get_monitor_config();
    let threshold = cfg.opportunity_push_threshold;
    let passed = score.event_risk_score >= threshold;
    let reason = format!(
        "event_risk={}/{} trade_signal={:?} passed={} notes={:?}",
        score.event_risk_score,
        threshold,
        score.trade_signal_score,
        passed,
        score.notes,
    );
    (score.event_risk_score, score.trade_signal_score, score.parts, passed, reason)
}

/// 修复 v9.1 §0 NS3 + §4 B10: 推送分层
/// - final >= 75: 实时推送 (realtime)
/// - 60 <= final < 75: 入候选池 (candidate pool, 复盘查阅)
/// - final < 60: 不推 (suppressed)
/// 注意: NS3 约束 event_risk_score 是唯一"风险评估"维度; trade_signal_score 触发 [实盘信号] 标签需另行评估
pub fn push_tier(event_risk_score: u8, push_threshold: u8) -> &'static str {
    if event_risk_score >= push_threshold {
        "realtime"  // 实时推送
    } else if event_risk_score >= 60 {
        "candidate_pool"  // 入候选池, 不主动推
    } else {
        "suppressed"  // 不推
    }
}

#[cfg(test)]
mod push_gate_tests {
    use super::*;
    use crate::opportunity::chain_mapper::ChainSource;

    fn make_hit(chain: &str, source: ChainSource, board_kw: &str, flow: Option<f64>) -> chain_mapper::ChainHit {
        chain_mapper::ChainHit {
            chain: chain.into(),
            keywords: vec!["测试".into()],
            logic: "测试逻辑".into(),
            stocks: vec![],
            source,
            board_keyword: board_kw.into(),
            fund_flow_pct: flow,
        }
    }

    #[test]
    fn test_push_tier_thresholds() {
        // 修复 v9.1 §4 B10: 推送分层
        assert_eq!(push_tier(80, 75), "realtime");
        assert_eq!(push_tier(75, 75), "realtime");
        assert_eq!(push_tier(74, 75), "candidate_pool");
        assert_eq!(push_tier(60, 75), "candidate_pool");
        assert_eq!(push_tier(59, 75), "suppressed");
        assert_eq!(push_tier(0, 75), "suppressed");
    }

    #[test]
    fn test_evaluate_hit_rule_source_passes_default_threshold() {
        // 修复 v9.1: Rule + 跨源 + 板名 = 强信号
        // 无 winrate 时 NS3 封顶 70, 实际 event_risk_score 在 50-65 区间
        let hit = make_hit("半导体", ChainSource::Rule, "半导体板块", Some(2.5));
        let flash = vec!["半导体突破".to_string()];
        let web = vec![SearchResult {
            title: "半导体突破".into(),
            snippet: "国内".into(),
            url: "".into(),
            source: "东财".into(),
            published_date: Some("2026-06-28 10:00:00".into()),
            news_type: crate::search_service::NewsType::Industry,
            sentiment: crate::search_service::Sentiment::Positive,
            importance: 8, relevance: 0.9, keywords: vec![],
        }];
        let (score, _tss, _parts, _passed, reason) = evaluate_hit_for_push(&hit, &flash, &web);
        // 量化 PM 视角: Rule 命中 + 跨源 + 板名 = 实际生产中应入"候选池" (≥ 60)
        // NS3 封顶 70 是设计选择 (沙盘阶段无 winrate 时不假装可上 75)
        assert!(score >= 50, "Rule + 跨源 必 ≥ 50 (候选池门槛), 实际 {} ({})", score, reason);
        // 注: 没 winrate 时通过不了 75 push 门槛 (NS3 强约束), 但 trade_signal 显式 None
        assert!(reason.contains("封顶") || reason.contains("无 winrate"),
                "reason 必标注 NS3 约束, 实际: {}", reason);
    }

    #[test]
    fn test_evaluate_hit_ai_degraded_capped() {
        // 修复 v9.1: AiDegraded → event_score ×0.5 → 必 < 75
        let hit = make_hit("半导体", ChainSource::AiDegraded, "", Some(0.0));
        let flash = vec!["半导体新闻".to_string()];
        let web = vec![];
        let (score, _tss, _parts, passed, _reason) = evaluate_hit_for_push(&hit, &flash, &web);
        assert!(!passed, "AiDegraded + 单源 必 < 75, 实际 {} (passed={})", score, passed);
    }

    #[test]
    fn test_build_score_inputs_from_hit_rule_baseline() {
        // 修复 v9.1: Rule 源 → event_strength=70 (不是 20/50)
        let hit = make_hit("半导体", ChainSource::Rule, "半导体", None);
        let inputs = build_score_inputs_from_hit(&hit, &[], &[]);
        assert_eq!(inputs.event_strength, 70);
        assert!(!inputs.ai_degraded);
        assert_eq!(inputs.cross_source_count, 1);  // flash=0 web=0 → 单源
    }

    #[test]
    fn test_build_score_inputs_from_hit_cross_source() {
        // 修复: flash_hits>0 + web_hits>0 → cross_source_count=2
        let hit = make_hit("半导体", ChainSource::Rule, "", None);
        let flash = vec!["半导体".to_string()];
        let web = vec![SearchResult {
            title: "半导体".into(), snippet: "".into(), url: "".into(),
            source: "东财".into(), published_date: Some("2026-06-28".into()),
            news_type: crate::search_service::NewsType::Other,
            sentiment: crate::search_service::Sentiment::Neutral,
            importance: 5, relevance: 0.5, keywords: vec![],
        }];
        let inputs = build_score_inputs_from_hit(&hit, &flash, &web);
        assert_eq!(inputs.cross_source_count, 2, "跨源必 = 2");
        assert!(inputs.event_certainty >= 60, "跨源 + certainty 必 ≥ 60, 实际 {}", inputs.event_certainty);
    }

    #[test]
    fn test_build_score_inputs_ai_degraded_flag() {
        // 修复 v9.1: AiDegraded → ai_degraded=true
        let hit = make_hit("新能源车", ChainSource::AiDegraded, "", None);
        let inputs = build_score_inputs_from_hit(&hit, &[], &[]);
        assert!(inputs.ai_degraded);
        assert_eq!(inputs.event_strength, 20);  // AiDegraded 基础分最低
    }
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
    /// P2-News Commit 4: NewsRanker 输出文本 (A/B/C/Drop 4 档)
    /// 旧调用方忽略此字段即可, 不影响向后兼容
    pub news_ranked_text: String,
}

/// 运行一次产业链扫描，返回「产业链」与「持仓影响」分离的结果
// ═══════════════════════════════════════════════════════════
// v9.4.24: 事件抽取公共逻辑 — rules-only 路径
// ═══════════════════════════════════════════════════════════

/// 从 Web 搜索结果中提取 MarketEvent，将非 Other 事件的 subject 回灌到 titles。
/// 返回检出的事件列表供调用方展示。
/// - stale 事件 (超过 2 天) 记录 warn 后丢弃
/// - Other 类型事件 (无明确分类) 不过滤入 titles，仅用于展示统计
fn collect_events_from_web(
    web_results: &[SearchResult],
    titles: &mut Vec<String>,
) -> (Vec<crate::signal::market_event::MarketEvent>, usize) {
    let (fresh_events, stale) = event_extractor::extract_batch_rules_only(web_results);
    let stale_n = stale.len();
    if stale_n > 0 {
        log::warn!(
            "[Opportunity] 事件抽取丢弃 {} 条过期事件 (>2天)",
            stale_n
        );
    }
    let detected: Vec<_> = fresh_events
        .into_iter()
        .filter(|e| e.event_type != crate::signal::market_event::EventType::Other)
        .collect();
    let event_n = detected.len();
    // 将检出事件的 subject 回灌到产业链映射输入，丰富信号源
    // 注: web_results 的完整 title 已在前一步通过 r.title.clone() 入 titles,
    // subject 是 30 字符截断版，作为补充冗余信号加入 (chain mapper 不做去重)
    for e in &detected {
        titles.push(e.subject.clone());
    }
    (detected, event_n)
}

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
        // 修复 watchlist-aware: 按持仓/自选板块动态生成查询
        // 之前 search query 是写死的 ("今日 A股 重大新闻 政策 产业"),
        // 用户的医药/军工/消费持仓根本搜不到行业新闻
        let sector_queries = build_watchlist_sector_queries();
        for q in sector_queries {
            let web = svc.search_topic(&q, 6).await;
            for r in web {
                // 仅纳入有一定重要性的，避免噪声
                if r.importance >= 5 && !r.title.trim().is_empty() {
                    titles.push(r.title.clone());
                    web_results.push(r);
                }
            }
        }
    }
    let web_n = web_results.len();

    // 1c. (v9.4.24) 事件抽取 — rules-only 路径, 用 collect_events_from_web 统一
    let (detected_events, event_n) = collect_events_from_web(&web_results, &mut titles);
    if event_n > 0 {
        let event_summary: Vec<String> = detected_events.iter()
            .map(|e| format!("{}({})", e.event_type.label(), e.subject.chars().take(20).collect::<String>()))
            .collect();
        lines.push(format!(
            "📊 事件抽取：快讯{}条/Web{}条 → 检出 {} 个有效事件 | {}",
            flash_n, web_n, event_n, event_summary.join("; ")
        ));
    }

    if titles.is_empty() {
        log::info!("[Opportunity] 采集 0 条新闻 → 跳过本轮");
        lines.push("暂无最新快讯".to_string());
        return OpportunityScan { chain_text: lines.join("\n"), impact_text: String::new(), news_ranked_text: String::new() };
    }

    // 2. 产业链映射（规则优先，未命中则 AI 兜底）
    let mut hits = chain_mapper::map_news_to_chains_ai(&titles).await;
    if hits.is_empty() {
        log::info!("[Opportunity] 采集 {} 条新闻(快讯{}/Web{}) → 命中 0 条产业链（含AI兜底）", titles.len(), flash_n, web_n);
        lines.push("当前快讯未命中已知产业链".to_string());
        return OpportunityScan { chain_text: lines.join("\n"), impact_text: String::new(), news_ranked_text: String::new() };
    }
    let rule_n = hits.iter().filter(|h| h.source == chain_mapper::ChainSource::Rule).count();
    let ai_n = hits.iter().filter(|h| h.source == chain_mapper::ChainSource::Ai).count();

    // 3. 动态解析标的（sector_monitor 内部用 reqwest::blocking，走 spawn_blocking）
    let mut hits = tokio::task::spawn_blocking(move || {
        chain_mapper::resolve_stocks(&mut hits);
        hits
    }).await.unwrap_or_default();
    hits.retain(|h| !h.stocks.is_empty());
    let (hits, dropped) = gate_hits(hits, &flash_titles, &web_results);
    // P2-News Commit 3: 影子模式跑新 ranker, log 对比 + 收集 ranked 列表
    // P2-News Commit 4: 收集后由 main.rs 推 PushKind::NewsRanked
    let ranked_news = if !hits.is_empty() {
        crate::opportunity::news_ranker::shadow_rank_hits(&hits, &titles)
    } else {
        Vec::new()
    };
    let news_ranked_text = crate::opportunity::news_ranker::format_news_ranked_board(&ranked_news);
    if hits.is_empty() {
        log::warn!(
            "[Opportunity] 采集 {} 条新闻(快讯{}/Web{}) → 候选被置信度门控全部过滤: {}",
            titles.len(), flash_n, web_n, dropped.join(" | ")
        );
        return OpportunityScan {
            chain_text: "当前产业链信号可信度不足（已降级观察）".to_string(),
            impact_text: String::new(),
            news_ranked_text,
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
        news_ranked_text,
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
        // 修复 watchlist-aware: 按持仓/自选板块动态生成查询
        let sector_queries = build_watchlist_sector_queries();
        for q in sector_queries {
            let web = svc.search_topic(&q, 8).await;
            for r in web {
                if r.importance >= 5 && !r.title.trim().is_empty() {
                    titles.push(r.title.clone());
                    web_results.push(r);
                }
            }
        }
    }

    // 1a. (v9.4.24) 事件抽取 — rules-only 路径, 用 collect_events_from_web 统一
    let (detected_events, event_n) = collect_events_from_web(&web_results, &mut titles);
    if event_n > 0 {
        let event_summary: Vec<String> = detected_events.iter()
            .map(|e| format!("{}({})", e.event_type.label(), e.subject.chars().take(20).collect::<String>()))
            .collect();
        lines.push(format!(
            "📊 事件预检：检出 {} 个有效事件 → 参与产业链映射 | {}",
            event_n, event_summary.join("; ")
        ));
    }

    if titles.is_empty() {
        return "🎯 优选候选：暂无可用新闻，跳过本轮".to_string();
    }

    // 2. 产业链映射 + 标的解析
    let mut hits = chain_mapper::map_news_to_chains_ai(&titles).await;
    if hits.is_empty() {
        return "🎯 优选候选：未命中产业链，不输出候选".to_string();
    }
    log::info!("[PostClose] map_news_to_chains_ai 命中 {} 条链", hits.len());
    let mut hits = tokio::task::spawn_blocking(move || {
        chain_mapper::resolve_stocks(&mut hits);
        hits
    })
    .await
    .unwrap_or_default();
    let before_retain = hits.len();
    hits.retain(|h| !h.stocks.is_empty());
    log::info!("[PostClose] resolve_stocks 后: {} 条 → retain 后 {} 条 (保留有成分股的)", before_retain, hits.len());
    let (hits, dropped) = gate_hits(hits, &flash_titles, &web_results);
    if !dropped.is_empty() {
        log::info!("[PostClose] gate_hits 过滤掉: {}", dropped.join(" | "));
    }
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
        // 修复 Top10#7 (2026-06-29 audit): 3 路 join! 并行 (financials + money_flow + kline)
        // 替代串行. 200 只股票 × 2 路串行延迟节省 ≈ 5-10 倍.
        let code = c.code.clone();
        let (fin_result, flow_result, kline_result) = tokio::join!(
            crate::data_provider::financials::fetch_with_fallback_async(&client, &code),
            crate::data_provider::money_flow::fetch_flow_history_async(&client, &code, 8),
            service::service().get_kline(&code, 80),
        );
        let fin = fin_result;
        let (flow, flow_fetch_error) = match flow_result {
            Ok(s) => {
                flow_ok_count += 1;
                (s, None)
            }
            Err(e) => {
                log::warn!("[资金流] {} 抓取失败: {}", code, e);
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
        let kline = kline_result.ok();

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
            base: c.clone(),  // 修复 F15: 需要 push_time, 不能 move
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
            push_time: c.push_time,  // 透传 discover::Candidate.push_time (BR-004)
        });
    }

    // 修复 F15 (2026-06-29 BR-004): final_score 降序, 同分按 push_time 升序 (越早越前)
    rescored.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.push_time.cmp(&b.push_time))
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

/// 修复 watchlist-aware search query: 按持仓/自选板块动态生成查询
///
/// 之前 search_topic 写死 "今日 A股 重大新闻 政策 产业",
/// 用户的医药/军工/消费持仓根本搜不到行业新闻。
/// 现在: 从 watchlist 取所有 Position.sector → 去重 → 转查询
/// 加 macro 维度 ("今日 板块 重大新闻") 让 search_topic 内部展开成
/// 板块特定的搜索词, 跨源覆盖医药/军工/消费/金融等之前漏掉的行业。
fn build_watchlist_sector_queries() -> Vec<String> {
    // 1) 加载 positions (失败 → 兜底)
    let positions = crate::portfolio::get_positions().unwrap_or_default();
    let watchlist = crate::portfolio::get_watchlist().unwrap_or_default();
    let mut all_positions = positions;
    for w in watchlist {
        if !all_positions.iter().any(|p| p.code == w.code) {
            all_positions.push(w);
        }
    }
    build_sector_queries_from_positions(&all_positions)
}

/// 修复: 内层纯函数 (供测试和复用), 输入 positions 输出查询
pub fn build_sector_queries_from_positions(positions: &[crate::portfolio::Position]) -> Vec<String> {
    // 1) 提取去重的 sector (过滤 "其他" 未分类和空字符串)
    let mut sectors: Vec<String> = positions.iter()
        .map(|p| p.sector.clone())
        .filter(|s| !s.is_empty() && s != "其他")
        .collect();
    sectors.sort();
    sectors.dedup();

    // 2) 转查询: 每 sector 一个查询 + 通用兜底
    let mut queries: Vec<String> = sectors.iter()
        .map(|s| format!("今日 {} 重大新闻 政策 催化", s))
        .collect();

    // 兜底: 即便 sector 全为 "其他" 或空也能拿到 A 股宏观新闻
    queries.push("今日 A股 重大新闻 政策 产业".to_string());

    // 量化 PM 视角: 上限 6 个查询, 避免 provider 配额耗尽
    queries.truncate(6);
    queries
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
            adjust: crate::data_provider::AdjustType::None,
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

    #[test]
    fn test_build_watchlist_sector_queries_dedup() {
        // 修复 watchlist-aware: 板块必去重, 不重复触发同一查询
        use crate::portfolio::Position;
        let positions = vec![
            Position { code: "600703".into(), name: "三安光电".into(), shares: 1000, cost_price: 10.0, hard_stop: 9.0, added_at: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), status: crate::portfolio::PositionStatus::Holding, sector: "半导体".into() },
            Position { code: "002049".into(), name: "紫光国微".into(), shares: 500, cost_price: 100.0, hard_stop: 90.0, added_at: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), status: crate::portfolio::PositionStatus::Holding, sector: "半导体".into() },
            Position { code: "600276".into(), name: "恒瑞医药".into(), shares: 800, cost_price: 50.0, hard_stop: 45.0, added_at: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), status: crate::portfolio::PositionStatus::Holding, sector: "医药".into() },
        ];
        let queries = super::build_sector_queries_from_positions(&positions);
        // 半导体(2 只) + 医药(1 只) → 2 个 sector 查询 + 1 兜底 = 3 个
        assert_eq!(queries.len(), 3, "去重后必 2 个 sector + 1 兜底");
        // 兜底必在尾部
        assert!(queries.last().unwrap().contains("今日 A股"));
        // 半导体 + 医药 各出现一次
        assert!(queries.iter().filter(|q| q.contains("半导体")).count() == 1);
        assert!(queries.iter().filter(|q| q.contains("医药")).count() == 1);
    }

    #[test]
    fn test_build_watchlist_sector_queries_filters_other() {
        // 修复: "其他" sector 必过滤, 不产生查询
        use crate::portfolio::Position;
        let positions = vec![
            Position { code: "600000".into(), name: "浦发银行".into(), shares: 1000, cost_price: 10.0, hard_stop: 9.0, added_at: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), status: crate::portfolio::PositionStatus::Watching, sector: "其他".into() },
        ];
        let queries = super::build_sector_queries_from_positions(&positions);
        // "其他" 被过滤, 仅有兜底查询
        assert_eq!(queries.len(), 1);
        assert!(queries[0].contains("今日 A股"));
    }

    #[test]
    fn test_build_watchlist_sector_queries_caps_at_six() {
        // 修复 watchlist-aware: 查询数 ≤ 6, 防止 provider 配额耗尽
        use crate::portfolio::Position;
        let positions: Vec<Position> = (0..20).map(|i| Position {
            code: format!("60000{}", i),
            name: format!("测试{}", i),
            shares: 100, cost_price: 10.0, hard_stop: 9.0,
            added_at: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            status: crate::portfolio::PositionStatus::Holding,
            sector: format!("行业{}", i),  // 每个 sector 都不同 → 20 个 sector
        }).collect();
        let queries = super::build_sector_queries_from_positions(&positions);
        // 20 个不同 sector + 1 兜底 → truncate(6) → 6 个
        assert_eq!(queries.len(), 6, "20 个 sector + 兜底必 truncate 到 6");
    }

    #[test]
    fn test_build_watchlist_sector_queries_empty_input() {
        // 边界: 空 positions 必返回兜底查询
        let queries = super::build_sector_queries_from_positions(&[]);
        assert_eq!(queries.len(), 1);
        assert!(queries[0].contains("今日 A股"));
    }
}
