//! Opportunity Context — 产业链挖掘 + 机会发现。
//!
//! 新闻事件 → 产业链映射 → 持仓影响评估 → 新标的推荐。

pub mod chain_mapper;
pub mod impact;
pub mod discover;

use crate::portfolio;

/// 运行一次产业链扫描，返回格式化文本
pub async fn run_opportunity_scan() -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("📡 产业链扫描（{}）", chrono::Local::now().format("%H:%M")));
    lines.push("━━━━━━━━━━━━━━━━━━━━━━━━".to_string());

    // 1. 获取最新快讯标题
    let svc = crate::search_service::get_search_service();
    let mut titles = svc.fetch_flash_titles(30).await;
    let flash_n = titles.len();

    // 1b. (T6a) Web 搜索补充今日重大新闻维度（失败容忍，不阻断；数据红线 2.1 不伪造）
    let mut web_n = 0;
    if svc.is_available() {
        let web = svc.search_topic("今日 A股 重大新闻 政策 产业", 8).await;
        for r in web {
            // 仅纳入有一定重要性的，避免噪声
            if r.importance >= 5 && !r.title.trim().is_empty() {
                titles.push(r.title);
                web_n += 1;
            }
        }
    }

    if titles.is_empty() {
        log::info!("[Opportunity] 采集 0 条新闻 → 跳过本轮");
        lines.push("暂无最新快讯".to_string());
        return lines.join("\n");
    }

    // 2. 产业链映射（规则优先，未命中则 AI 兜底）
    let mut hits = chain_mapper::map_news_to_chains_ai(&titles).await;
    if hits.is_empty() {
        log::info!("[Opportunity] 采集 {} 条新闻(快讯{}/Web{}) → 命中 0 条产业链（含AI兜底）", titles.len(), flash_n, web_n);
        lines.push("当前快讯未命中已知产业链".to_string());
        return lines.join("\n");
    }
    let rule_n = hits.iter().filter(|h| h.source == chain_mapper::ChainSource::Rule).count();
    let ai_n = hits.iter().filter(|h| h.source == chain_mapper::ChainSource::Ai).count();

    // 3. 动态解析标的（sector_monitor 内部用 reqwest::blocking，走 spawn_blocking）
    let mut hits = tokio::task::spawn_blocking(move || {
        chain_mapper::resolve_stocks(&mut hits);
        hits
    }).await.unwrap_or_default();
    hits.retain(|h| !h.stocks.is_empty());
    if hits.is_empty() {
        log::info!("[Opportunity] 采集 {} 条新闻(快讯{}/Web{}) → 命中产业链(规则{}/AI{}) 但无可用标的", titles.len(), flash_n, web_n, rule_n, ai_n);
        return "当前产业链无可用标的".to_string();
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

    // 持仓影响
    if !impacts.is_empty() {
        lines.push(String::new());
        lines.push("📌 你的持仓影响：".to_string());
        for imp in &impacts {
            lines.push(format!("  {} {}({}) — {}: {}",
                imp.direction.emoji(), imp.name, imp.code,
                imp.direction.label(), imp.reason));
        }
    }

    // 新标的推荐
    let candidates = discover::discover(&hits, &our_codes, 3);
    if !candidates.is_empty() {
        lines.push(String::new());
        lines.push("🎯 值得关注：".to_string());
        for c in &candidates {
            let note = if c.price_note.is_empty() { String::new() } else { format!(" [{}]", c.price_note) };
            lines.push(format!("  {}({}) — {}：{}{}", c.name, c.code, c.chain, c.logic, note));
            // 影子盘留痕（合规可追溯，AGENTS.md 2.7）：机会以"看多"记入 prediction_tracker
            crate::monitor::prediction::save_prediction(
                Some(&c.chain), Some(&c.code), "看多", 60.0, Some(&c.logic),
            );
        }
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
        "[Opportunity] 采集 {} 条新闻(快讯{}/Web{}) → 命中产业链 {} 条(规则{}/AI{}) → 推送 {} 个机会",
        titles.len(), flash_n, web_n, hits.len(), rule_n, ai_n, candidates.len()
    );

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
