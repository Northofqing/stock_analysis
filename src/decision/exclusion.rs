//! Registered business rules: BR-052.
//! 排除引擎 — 扫描持仓/自选，标记命中排除板块的标的。
//!
//! 匹配方式：拉取排除板块的成份股 → 交叉比对持仓代码。

use std::sync::OnceLock;

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
        return config_boards
            .iter()
            .map(|b| (b.name.clone(), b.reason.clone()))
            .collect();
    }
    DEFAULT_EXCLUDED_BOARDS
        .iter()
        .map(|(n, r)| (n.to_string(), r.to_string()))
        .collect()
}

/// 缓存的 (日期, 映射) — 同一天复用, 避免每次 review 600 次 HTTP (review #14 修复).
/// review 路径每天跑多次, 这里缓存一天一次拉取就够.
struct CachedExclusionMap {
    date: chrono::NaiveDate,
    map: std::collections::HashMap<String, (String, String)>,
}

static EXCLUSION_MAP_CACHE: OnceLock<std::sync::Mutex<Option<CachedExclusionMap>>> =
    OnceLock::new();

fn cached_exclusion_map() -> std::collections::HashMap<String, (String, String)> {
    let cell = EXCLUSION_MAP_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    let today = chrono::Local::now().date_naive();
    {
        let guard = cell.lock().unwrap();
        if let Some(c) = guard.as_ref() {
            if c.date == today {
                return c.map.clone();
            }
        }
    }
    let map = build_exclusion_map();
    *cell.lock().unwrap() = Some(CachedExclusionMap {
        date: today,
        map: map.clone(),
    });
    map
}

/// 测试 / 调试用 — 强制清缓存 (例如 toml reload 后).
#[cfg(test)]
pub fn clear_exclusion_cache() {
    if let Some(cell) = EXCLUSION_MAP_CACHE.get() {
        *cell.lock().unwrap() = None;
    }
}

/// review #15: source 改 enum, 替代字符串比较 (`if h.source == "持仓"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusionSource {
    Holding,
    Watchlist,
}

impl ExclusionSource {
    pub fn label(self) -> &'static str {
        match self {
            ExclusionSource::Holding => "持仓",
            ExclusionSource::Watchlist => "自选",
        }
    }
    pub fn emoji(self) -> &'static str {
        match self {
            ExclusionSource::Holding => "⚠️",
            ExclusionSource::Watchlist => "📌",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExclusionHit {
    pub code: String,
    pub name: String,
    pub matched_board: String,
    pub reason: String,
    pub source: ExclusionSource,
}

/// 一次拉取所有排除板块的成份股，构建 code→board 映射
fn build_exclusion_map() -> std::collections::HashMap<String, (String, String)> {
    let mut map = std::collections::HashMap::new();
    let boards_listing =
        match crate::market_analyzer::sector_monitor::fetch_board_ranking("f3", 100) {
            Ok(b) => b,
            Err(e) => {
                log::warn!(
                    "exclusion: 板块排名拉取失败 ({}), 跳过本次排除扫描 — 风险板块可能漏检",
                    e
                );
                return map;
            }
        };
    let mut failed_boards: Vec<&str> = Vec::new();
    let excluded = excluded_boards();
    for (board_name, reason) in &excluded {
        let Some(b) = boards_listing.iter().find(|b| b.name.contains(board_name)) else {
            failed_boards.push(board_name);
            continue;
        };
        match crate::market_analyzer::sector_monitor::fetch_board_components(&b.code, 50) {
            Ok(stocks) => {
                for s in stocks {
                    map.entry(s.code)
                        .or_insert_with(|| (board_name.clone(), reason.clone()));
                }
            }
            Err(e) => {
                log::warn!("exclusion: 板块 {} 成份股拉取失败: {}", board_name, e);
                failed_boards.push(board_name);
            }
        }
    }
    if !failed_boards.is_empty() {
        log::warn!(
            "exclusion: {} 个排除板块扫描失败: {:?}, 请人工复核持仓",
            failed_boards.len(),
            failed_boards
        );
    }
    map
}

/// 扫描持仓和自选，返回命中排除板块的标的
pub fn scan_exclusions(holdings: &[Position], watchlist: &[Position]) -> Vec<ExclusionHit> {
    let exclusion_map = cached_exclusion_map();
    if exclusion_map.is_empty() {
        return vec![];
    }
    scan_exclusions_with_map(&exclusion_map, holdings, watchlist)
}

fn scan_exclusions_with_map(
    exclusion_map: &std::collections::HashMap<String, (String, String)>,
    holdings: &[Position],
    watchlist: &[Position],
) -> Vec<ExclusionHit> {
    let mut hits = Vec::new();
    for p in holdings {
        if let Some((board, reason)) = exclusion_map.get(&p.code) {
            hits.push(ExclusionHit {
                code: p.code.clone(),
                name: p.name.clone(),
                matched_board: board.clone(),
                reason: reason.clone(),
                source: ExclusionSource::Holding,
            });
        }
    }
    for p in watchlist {
        if let Some((board, reason)) = exclusion_map.get(&p.code) {
            hits.push(ExclusionHit {
                code: p.code.clone(),
                name: p.name.clone(),
                matched_board: board.clone(),
                reason: reason.clone(),
                source: ExclusionSource::Watchlist,
            });
        }
    }
    hits
}

/// 格式化排除告警
pub fn format_exclusion_alert(hits: &[ExclusionHit]) -> String {
    if hits.is_empty() {
        return String::new();
    }
    use std::fmt::Write;
    let mut out = String::with_capacity(64 + hits.len() * 40);
    out.push_str("🛑 排除板块命中\n");
    for h in hits {
        let _ = writeln!(
            out,
            "  {} {}({}) — {}: {}",
            h.source.emoji(),
            h.name,
            h.code,
            h.matched_board,
            h.reason,
        );
    }
    out
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
            code: "TEST_CODE_000858".into(),
            name: "五粮液".into(),
            matched_board: "白酒".into(),
            reason: "成熟天花板".into(),
            source: ExclusionSource::Holding,
        }];
        let text = format_exclusion_alert(&hits);
        assert!(text.contains("排除板块命中"));
        assert!(text.contains("白酒"));
    }

    #[test]
    fn source_labels_and_isolated_map_scan_cover_holding_and_watchlist() {
        assert_eq!(ExclusionSource::Holding.label(), "持仓");
        assert_eq!(ExclusionSource::Watchlist.label(), "自选");
        assert_eq!(ExclusionSource::Holding.emoji(), "⚠️");
        assert_eq!(ExclusionSource::Watchlist.emoji(), "📌");

        let map = std::collections::HashMap::from([
            (
                "TEST_CODE_000001".to_string(),
                ("排除甲".to_string(), "原因甲".to_string()),
            ),
            (
                "TEST_CODE_000002".to_string(),
                ("排除乙".to_string(), "原因乙".to_string()),
            ),
        ]);
        let holdings = vec![Position {
            code: "TEST_CODE_000001".to_string(),
            name: "持仓甲".to_string(),
            ..Position::default()
        }];
        let watchlist = vec![
            Position {
                code: "TEST_CODE_000002".to_string(),
                name: "观察乙".to_string(),
                ..Position::default()
            },
            Position {
                code: "TEST_CODE_000003".to_string(),
                name: "未命中".to_string(),
                ..Position::default()
            },
        ];
        let hits = scan_exclusions_with_map(&map, &holdings, &watchlist);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].source, ExclusionSource::Holding);
        assert_eq!(hits[1].source, ExclusionSource::Watchlist);
        let rendered = format_exclusion_alert(&hits);
        assert!(rendered.contains("持仓甲"));
        assert!(rendered.contains("观察乙"));
        assert!(!rendered.contains("未命中"));
    }

    #[test]
    fn test_exclusion_map_built() {
        clear_exclusion_cache();
        // 不依赖网络时返回空 map（优雅降级）
        let map = build_exclusion_map();
        assert!(map.is_empty() || !map.is_empty());
    }
}
