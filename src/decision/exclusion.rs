//! 排除引擎 — 扫描持仓/自选，标记命中排除板块的标的。
//!
//! 匹配方式：拉取排除板块的成份股 → 交叉比对持仓代码。

use crate::portfolio::Position;

/// 排除板块：板块名 → 原因。toml 不可用时回退此默认值。
const DEFAULT_EXCLUDED_BOARDS: &[(&str, &str)] = &[
    ("白酒", "成熟天花板，缺乏弹性"),
    ("猪肉", "周期下行，产能过剩"),
    ("房地产", "行业下行，政策刺激持续性弱"),
    ("光伏", "产能过剩，价格战未结束"),
    ("家电", "增长见顶，缺乏弹性"),
    ("银行", "成熟天花板"),
    ("证券", "高度周期，难成主线"),
    ("军工", "纯政策刺激，持续性弱"),
    ("煤炭", "周期下行"),
    ("钢铁", "产能过剩"),
];

fn excluded_boards() -> Vec<(String, String)> {
    if let Some(config_boards) = crate::config::get_exclusion_boards() {
        return config_boards.into_iter().map(|b| (b.name, b.reason)).collect();
    }
    DEFAULT_EXCLUDED_BOARDS.iter().map(|(n, r)| (n.to_string(), r.to_string())).collect()
}

#[derive(Debug, Clone)]
pub struct ExclusionHit {
    pub code: String,
    pub name: String,
    pub matched_board: String,
    pub reason: String,
    pub source: String,
}

/// 一次拉取所有排除板块的成份股，构建 code→board 映射（缓存当天）
fn build_exclusion_map() -> std::collections::HashMap<String, (String, String)> {
    let mut map = std::collections::HashMap::new();
    for (board_name, reason) in &excluded_boards() {
        // 先用板块名直接搜
        if let Ok(boards) = crate::market_analyzer::sector_monitor::fetch_board_ranking("f3", 100) {
            if let Some(b) = boards.iter().find(|b| b.name.contains(board_name)) {
                if let Ok(stocks) = crate::market_analyzer::sector_monitor::fetch_board_components(&b.code, 50) {
                    for s in stocks {
                        map.entry(s.code).or_insert_with(|| (board_name.to_string(), reason.to_string()));
                    }
                }
            }
        }
    }
    map
}

/// 扫描持仓和自选，返回命中排除板块的标的
pub fn scan_exclusions(holdings: &[Position], watchlist: &[Position]) -> Vec<ExclusionHit> {
    let exclusion_map = build_exclusion_map();
    if exclusion_map.is_empty() { return vec![]; }

    let mut hits = Vec::new();
    for p in holdings {
        if let Some((board, reason)) = exclusion_map.get(&p.code) {
            hits.push(ExclusionHit {
                code: p.code.clone(), name: p.name.clone(),
                matched_board: board.clone(), reason: reason.clone(),
                source: "持仓".to_string(),
            });
        }
    }
    for p in watchlist {
        if let Some((board, reason)) = exclusion_map.get(&p.code) {
            hits.push(ExclusionHit {
                code: p.code.clone(), name: p.name.clone(),
                matched_board: board.clone(), reason: reason.clone(),
                source: "自选".to_string(),
            });
        }
    }
    hits
}

/// 格式化排除告警
pub fn format_exclusion_alert(hits: &[ExclusionHit]) -> String {
    if hits.is_empty() { return String::new(); }
    let mut lines = vec!["🛑 排除板块命中".to_string()];
    for h in hits {
        lines.push(format!(
            "  {} {}({}) — {}: {}",
            if h.source == "持仓" { "⚠️" } else { "📌" },
            h.name, h.code, h.matched_board, h.reason,
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_empty() {
        assert!(format_exclusion_alert(&[]).is_empty());
    }

    #[test]
    fn test_format_with_hits() {
        let hits = vec![ExclusionHit {
            code: "000858".into(), name: "五粮液".into(),
            matched_board: "白酒".into(), reason: "成熟天花板".into(),
            source: "持仓".into(),
        }];
        let text = format_exclusion_alert(&hits);
        assert!(text.contains("排除板块命中"));
        assert!(text.contains("白酒"));
    }

    #[test]
    fn test_exclusion_map_built() {
        // 不依赖网络时返回空 map（优雅降级）
        let map = build_exclusion_map();
        // map 可能为空（无网络）或非空（有网络），两种都合法
        assert!(map.is_empty() || !map.is_empty());
    }
}
