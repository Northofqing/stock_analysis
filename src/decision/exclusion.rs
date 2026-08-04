//! Registered business rules: BR-052, BR-188.
//! 排除引擎 — 扫描持仓/自选，标记命中排除板块的标的。
//!
//! 匹配方式：对完整持仓/自选代码集合读取 Magic TDX memberships，再按排除配置匹配。

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use crate::data_gateway::{BoardDataGateway, BoardKind, BoardMembershipRecord, GatewayBatch};
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

/// 缓存必须绑定日期和完整、稳定排序后的目标代码集合。
struct CachedExclusionMap {
    date: chrono::NaiveDate,
    codes: Vec<String>,
    map: HashMap<String, (String, String)>,
}

static EXCLUSION_MAP_CACHE: OnceLock<std::sync::Mutex<Option<CachedExclusionMap>>> =
    OnceLock::new();

fn cached_exclusion_map(codes: &[String]) -> Result<HashMap<String, (String, String)>, String> {
    let today = chrono::Local::now().date_naive();
    cached_exclusion_map_for_date(today, codes, build_exclusion_map)
}

fn cached_exclusion_map_for_date<F>(
    today: chrono::NaiveDate,
    codes: &[String],
    build: F,
) -> Result<HashMap<String, (String, String)>, String>
where
    F: FnOnce(&[String]) -> Result<HashMap<String, (String, String)>, String>,
{
    let cell = EXCLUSION_MAP_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    {
        let guard = cell
            .lock()
            .map_err(|_| "exclusion membership cache mutex poisoned".to_string())?;
        if let Some(c) = guard.as_ref() {
            if c.date == today && c.codes == codes {
                return Ok(c.map.clone());
            }
        }
    }
    let map = build(codes)?;
    *cell
        .lock()
        .map_err(|_| "exclusion membership cache mutex poisoned".to_string())? =
        Some(CachedExclusionMap {
            date: today,
            codes: codes.to_vec(),
            map: map.clone(),
        });
    Ok(map)
}

/// 测试 / 调试用 — 强制清缓存 (例如 toml reload 后).
#[cfg(test)]
pub fn clear_exclusion_cache() {
    if let Some(cell) = EXCLUSION_MAP_CACHE.get() {
        if let Ok(mut guard) = cell.lock() {
            *guard = None;
        }
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

fn build_exclusion_map(codes: &[String]) -> Result<HashMap<String, (String, String)>, String> {
    let gateway = BoardDataGateway::production_tdx();
    build_exclusion_map_from_memberships(codes, &excluded_boards(), |code| {
        gateway
            .memberships_blocking(code)
            .map_err(|error| format!("code={code} memberships Gateway 失败: {error}"))
    })
}

fn build_exclusion_map_from_memberships<F>(
    codes: &[String],
    excluded: &[(String, String)],
    mut fetch: F,
) -> Result<HashMap<String, (String, String)>, String>
where
    F: FnMut(&str) -> Result<GatewayBatch<BoardMembershipRecord>, String>,
{
    let mut stable_codes = codes.to_vec();
    stable_codes.sort_unstable();
    stable_codes.dedup();
    if stable_codes != codes {
        return Err("exclusion target codes must be sorted and deduplicated".to_string());
    }

    let mut map = HashMap::new();
    for code in codes {
        let batch = fetch(code)?;
        let evidence = batch.evidence();
        let mut board_names = BTreeSet::new();
        match &batch {
            GatewayBatch::Available { records, .. } => {
                if records.is_empty() {
                    return Err(format!("code={code} memberships 返回非法 Available 空批次"));
                }
                for record in records {
                    if record.instrument_code != *code {
                        return Err(format!(
                            "code={code} memberships 身份不一致: returned={}",
                            record.instrument_code
                        ));
                    }
                    if !matches!(record.kind, BoardKind::Industry | BoardKind::Concept) {
                        continue;
                    }
                    if record.board_name.trim() != record.board_name
                        || record.board_name.is_empty()
                        || record.board_code.trim() != record.board_code
                        || record.board_code.is_empty()
                    {
                        return Err(format!(
                            "code={code} memberships 含非法板块身份: code={:?} name={:?}",
                            record.board_code, record.board_name
                        ));
                    }
                    board_names.insert(record.board_name.as_str());
                }
                log::info!(
                    "[exclusion][BR-188] code={} status=available records={} provider={:?} observed_at={} batch_id={}",
                    code,
                    records.len(),
                    evidence.provider,
                    evidence.observed_at,
                    evidence.batch_id
                );
            }
            GatewayBatch::VerifiedEmpty(_) => {
                log::info!(
                    "[exclusion][BR-188] code={} status=verified_empty provider={:?} observed_at={} batch_id={}",
                    code,
                    evidence.provider,
                    evidence.observed_at,
                    evidence.batch_id
                );
            }
        }

        if let Some((name, reason)) = excluded.iter().find(|(excluded_name, _)| {
            board_names
                .iter()
                .any(|actual_name| actual_name.contains(excluded_name.as_str()))
        }) {
            map.insert(code.clone(), (name.clone(), reason.clone()));
        }
    }
    Ok(map)
}

/// 扫描持仓和自选，返回命中排除板块的标的
pub fn scan_exclusions(
    holdings: &[Position],
    watchlist: &[Position],
) -> Result<Vec<ExclusionHit>, String> {
    let mut codes = holdings
        .iter()
        .chain(watchlist)
        .map(|position| position.code.clone())
        .collect::<Vec<_>>();
    codes.sort_unstable();
    codes.dedup();
    if codes.is_empty() {
        return Ok(Vec::new());
    }
    let exclusion_map = cached_exclusion_map(&codes)?;
    if exclusion_map.is_empty() {
        return Ok(Vec::new());
    }
    Ok(scan_exclusions_with_map(
        &exclusion_map,
        holdings,
        watchlist,
    ))
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
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    fn membership_batch(
        batch_id: &str,
        records: Vec<BoardMembershipRecord>,
    ) -> GatewayBatch<BoardMembershipRecord> {
        GatewayBatch::Available {
            records,
            evidence: crate::data_gateway::BatchEvidence {
                provider: magic_market_core::ProviderId::Tdx,
                source: "TEST_CODE_tdx-block-files".to_owned(),
                source_at: None,
                observed_at: "1785290400.000000000".to_owned(),
                batch_id: batch_id.to_owned(),
            },
        }
    }

    fn membership(
        instrument_code: &str,
        board_code: &str,
        board_name: &str,
        kind: BoardKind,
    ) -> BoardMembershipRecord {
        BoardMembershipRecord {
            instrument_code: instrument_code.to_owned(),
            board_code: board_code.to_owned(),
            board_name: board_name.to_owned(),
            kind,
        }
    }

    #[test]
    fn br188_membership_mapping_uses_config_order_and_excludes_region() {
        let codes = vec!["TEST_CODE_000001".to_owned(), "TEST_CODE_000002".to_owned()];
        let excluded = vec![
            ("银行".to_owned(), "银行原因".to_owned()),
            ("白酒".to_owned(), "白酒原因".to_owned()),
        ];
        let map = build_exclusion_map_from_memberships(&codes, &excluded, |code| {
            let records = match code {
                "TEST_CODE_000001" => vec![
                    membership(code, "tdx:concept:白酒概念", "白酒概念", BoardKind::Concept),
                    membership(code, "tdx:industry:银行", "银行", BoardKind::Industry),
                ],
                "TEST_CODE_000002" => vec![membership(
                    code,
                    "tdx:region:白酒产区",
                    "白酒产区",
                    BoardKind::Region,
                )],
                other => panic!("unexpected TEST_CODE identity {other}"),
            };
            Ok(membership_batch(
                &format!("TEST_CODE_batch_{code}"),
                records,
            ))
        })
        .unwrap();

        assert_eq!(
            map["TEST_CODE_000001"],
            ("银行".to_owned(), "银行原因".to_owned())
        );
        assert!(!map.contains_key("TEST_CODE_000002"));
    }

    #[test]
    fn br188_membership_mapping_rejects_partial_failure_and_identity_mismatch() {
        let codes = vec!["TEST_CODE_000001".to_owned(), "TEST_CODE_000002".to_owned()];
        let excluded = vec![("白酒".to_owned(), "测试原因".to_owned())];
        let error = build_exclusion_map_from_memberships(&codes, &excluded, |code| {
            if code == "TEST_CODE_000002" {
                return Err("TEST_CODE provider unavailable".to_owned());
            }
            Ok(membership_batch(
                "TEST_CODE_first",
                vec![membership(
                    code,
                    "tdx:concept:白酒",
                    "白酒",
                    BoardKind::Concept,
                )],
            ))
        })
        .unwrap_err();
        assert!(error.contains("provider unavailable"));

        let error = build_exclusion_map_from_memberships(&codes[..1], &excluded, |_| {
            Ok(membership_batch(
                "TEST_CODE_mismatch",
                vec![membership(
                    "TEST_CODE_000002",
                    "tdx:concept:白酒",
                    "白酒",
                    BoardKind::Concept,
                )],
            ))
        })
        .unwrap_err();
        assert!(error.contains("身份不一致"));
    }

    #[test]
    fn br188_cache_binds_date_and_exact_target_set_and_never_caches_failure() {
        clear_exclusion_cache();
        let date = chrono::Local::now().date_naive();
        let builds = AtomicUsize::new(0);
        let codes = vec!["TEST_CODE_000001".to_owned()];
        let first = cached_exclusion_map_for_date(date, &codes, |_| {
            builds.fetch_add(1, Ordering::Relaxed);
            Ok(HashMap::from([(
                "TEST_CODE_000001".to_string(),
                ("测试排除板块".to_string(), "测试原因".to_string()),
            )]))
        })
        .unwrap();
        let second = cached_exclusion_map_for_date(date, &codes, |_| {
            builds.fetch_add(1, Ordering::Relaxed);
            Ok(HashMap::new())
        })
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(builds.load(Ordering::Relaxed), 1);

        let expanded_codes = vec!["TEST_CODE_000001".to_owned(), "TEST_CODE_000002".to_owned()];
        let refreshed = cached_exclusion_map_for_date(date, &expanded_codes, |_| {
            builds.fetch_add(1, Ordering::Relaxed);
            Ok(HashMap::from([(
                "TEST_CODE_000002".to_string(),
                ("新增排除板块".to_string(), "新增原因".to_string()),
            )]))
        })
        .unwrap();
        assert!(refreshed.contains_key("TEST_CODE_000002"));
        assert_eq!(builds.load(Ordering::Relaxed), 2);

        clear_exclusion_cache();
        let failure_calls = AtomicUsize::new(0);
        assert!(cached_exclusion_map_for_date(date, &codes, |_| {
            failure_calls.fetch_add(1, Ordering::Relaxed);
            Err("TEST_CODE unavailable".to_owned())
        })
        .is_err());
        let recovered = cached_exclusion_map_for_date(date, &codes, |_| {
            failure_calls.fetch_add(1, Ordering::Relaxed);
            Ok(HashMap::new())
        })
        .unwrap();
        assert!(recovered.is_empty());
        assert_eq!(failure_calls.load(Ordering::Relaxed), 2);
        clear_exclusion_cache();
    }

    #[test]
    fn empty_target_scan_is_verified_empty_without_provider_access() {
        assert!(scan_exclusions(&[], &[]).unwrap().is_empty());
    }

    #[test]
    fn configured_or_default_exclusion_board_set_is_nonempty() {
        let boards = excluded_boards();
        assert!(!boards.is_empty());
        assert!(boards
            .iter()
            .all(|(name, reason)| !name.trim().is_empty() && !reason.trim().is_empty()));
    }
}
