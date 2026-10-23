//! 概念共现 / 同概念关联票（2026-10-18）。
//!
//! ## 为什么不用 `concept_cooccurrence` 表
//!
//! 那张表 0 行、没有写入方。我之前把它当成"这件事做不了"的证据——
//! 但 `stock_concepts` 存着 1265 只票的完整概念标签（JSON 数组），
//! 共现关系可以直接从它算出来，不需要预计算表。
//!
//! ## 共现强度
//!
//! **共享概念数**。两只票共享的概念越多，联动概率越高。
//! 这是"关联公司"最弱的一层：同概念 ≠ 上下游，但能抓住板块联动。
//! 真正的产业链关系需要 Windows 侧提供（见 grpc_handoffs 规格单）。
//!
//! ## 边界
//!
//! - 只读 `stock_concepts`，不写任何表
//! - `min_shared` 默认用 2：只共享 1 个概念时噪声太大
//!   （"融资融券""MSCI成份""陆股通重"这类标签几乎人人都有）
//! - 名称查不到就留 None，显示时只有代码，不编造

use std::collections::HashSet;

use diesel::prelude::*;
use diesel::sql_types::Text;

use crate::database::DatabaseManager;

/// 默认最少共享概念数。低于 2 时噪声过大（通用标签会命中大半市场）。
pub const DEFAULT_MIN_SHARED: usize = 2;

/// 通用标签 —— 几乎人人都有，不能用来判断业务关联。
///
/// 实测暴露：601811（新华文轩）的 16 个概念里，有 6 个是这种通用标签。
/// 门槛设 2 时会命中 272 只票，其中大半是靠"最近多板""养老金"凑出来的。
/// 这些标签描述的是**交易状态或资金属性**，不是业务关联。
const GENERIC_TAGS: &[&str] = &[
    "融资融券",
    "MSCI成份",
    "陆股通重",
    "养老金",
    "QFII重仓",
    "QFII新进",
    "最近多板",
    "最近情绪",
    "近期强势",
    "昨日涨停",
    "昨日跌停",
    "昨日连板",
    "低安全分",
    "高市盈率",
    "非周期股",
    "含H股",
    "含B股",
    "预盈预增",
    "预亏预减",
    "破净股",
    "次新股",
    "超跌反弹",
    "举牌",
    "股权转让",
];

fn is_generic(tag: &str) -> bool {
    GENERIC_TAGS.iter().any(|generic| generic == &tag)
}

#[derive(Debug, Clone)]
pub struct Peer {
    pub code: String,
    pub name: Option<String>,
    pub shared: Vec<String>,
}

impl Peer {
    pub fn render(&self) -> String {
        let label = match &self.name {
            Some(name) if !name.is_empty() => format!("{} {}", self.code, name),
            _ => self.code.clone(),
        };
        format!("{}  [{}]", label, self.shared.join("、"))
    }
}

#[derive(QueryableByName)]
struct ConceptRow {
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    concepts: String,
}

fn resolve_name(conn: &mut SqliteConnection, code: &str) -> Option<String> {
    #[derive(QueryableByName)]
    struct NameRow {
        #[diesel(sql_type = Text)]
        name: String,
    }
    const SOURCES: [&str; 4] = [
        "stock_position",
        "user_position_snapshot_item",
        "pushed_stocks",
        "catalyst_watchlist_daily",
    ];
    for table in SOURCES {
        let sql = format!(
            "SELECT name FROM {table} \
             WHERE code = ? AND name IS NOT NULL AND name <> '' \
             ORDER BY rowid DESC LIMIT 1"
        );
        if let Ok(row) = diesel::sql_query(sql)
            .bind::<Text, _>(code)
            .get_result::<NameRow>(conn)
        {
            return Some(row.name);
        }
    }
    None
}

/// 一次装载 `stock_concepts`（全表一次查询）。
///
/// 2026-10-24：原 `peers_of` 每调用一次就全表扫描一次。P3.3 每 30 秒一轮、
/// 观察池十几只票 → 每轮十几次全表扫描，直接压主库。改为「装一次、按票分组」。
pub struct ConceptIndex {
    parsed: Vec<(String, HashSet<String>)>,
    malformed: usize,
}

impl ConceptIndex {
    pub fn load(db: &DatabaseManager) -> Result<Self, String> {
        let mut conn = db.get_conn().map_err(|e| e.to_string())?;
        let rows: Vec<ConceptRow> = diesel::sql_query("SELECT code, concepts FROM stock_concepts")
            .load(&mut conn)
            .map_err(|e| e.to_string())?;
        let mut parsed: Vec<(String, HashSet<String>)> = Vec::new();
        let mut malformed = 0usize;
        for row in rows {
            match serde_json::from_str::<Vec<String>>(&row.concepts) {
                Ok(list) => parsed.push((row.code, list.into_iter().collect())),
                Err(_) => malformed += 1,
            }
        }
        if malformed > 0 {
            log::warn!("[概念共现] {malformed} 行 concepts 不是合法 JSON 数组，已跳过");
        }
        Ok(Self { parsed, malformed })
    }

    pub fn malformed(&self) -> usize {
        self.malformed
    }

    /// 某只票的同概念票（不带名称：调用方一般已有行情里的名称）。
    fn peers_in(&self, code: &str, min_shared: usize) -> Vec<Peer> {
        let Some((_, target)) = self.parsed.iter().find(|(candidate, _)| candidate == code) else {
            return Vec::new();
        };
        let mut peers: Vec<Peer> = self
            .parsed
            .iter()
            .filter(|(candidate, _)| candidate != code)
            .filter_map(|(candidate, set)| {
                let mut shared: Vec<String> = target
                    .intersection(set)
                    .filter(|tag| !is_generic(tag))
                    .cloned()
                    .collect();
                if shared.len() < min_shared {
                    return None;
                }
                shared.sort();
                Some(Peer {
                    code: candidate.clone(),
                    name: None,
                    shared,
                })
            })
            .collect();
        peers.sort_by(|a, b| {
            b.shared
                .len()
                .cmp(&a.shared.len())
                .then_with(|| a.code.cmp(&b.code))
        });
        peers
    }

    /// 一批票的同概念票：`subject -> peers`。全表只装载一次。
    pub fn peers_many(
        &self,
        codes: &[String],
        min_shared: usize,
    ) -> std::collections::HashMap<String, Vec<Peer>> {
        codes
            .iter()
            .map(|code| (code.clone(), self.peers_in(code, min_shared)))
            .filter(|(_, peers)| !peers.is_empty())
            .collect()
    }
}

/// 给定一只票，返回共享概念数 >= `min_shared` 的其他票，按共享数降序。
pub fn peers_of(db: &DatabaseManager, code: &str, min_shared: usize) -> Result<Vec<Peer>, String> {
    let index = ConceptIndex::load(db)?;
    if !index.parsed.iter().any(|(candidate, _)| candidate == code) {
        return Err(format!("{code} 不在 stock_concepts 里"));
    }
    let mut peers = index.peers_in(code, min_shared);
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    for peer in &mut peers {
        peer.name = resolve_name(&mut conn, &peer.code);
    }
    Ok(peers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn shared_count_drives_ranking() {
        let target = set(&["光伏", "风电", "氢能源"]);
        let near = set(&["光伏", "风电", "储能"]);
        let far = set(&["光伏", "半导体"]);
        let unrelated = set(&["白酒"]);

        assert_eq!(target.intersection(&near).count(), 2);
        assert_eq!(target.intersection(&far).count(), 1);
        assert_eq!(target.intersection(&unrelated).count(), 0);
        // min_shared=2 时 far 和 unrelated 都应被过滤
        assert!(target.intersection(&far).count() < DEFAULT_MIN_SHARED);
    }

    #[test]
    fn default_threshold_is_two() {
        assert_eq!(DEFAULT_MIN_SHARED, 2);
    }

    #[test]
    fn peers_many_loads_once_and_groups_by_subject() {
        let index = ConceptIndex {
            parsed: vec![
                ("600001".to_string(), set(&["光伏", "风电"])),
                ("600002".to_string(), set(&["光伏", "风电", "储能"])),
                ("600003".to_string(), set(&["光伏", "融资融券"])),
                ("600004".to_string(), set(&["白酒"])),
            ],
            malformed: 0,
        };
        let map = index.peers_many(
            &["600001".to_string(), "600004".to_string()],
            DEFAULT_MIN_SHARED,
        );
        let first = map.get("600001").expect("600001 有同概念票");
        assert_eq!(first.len(), 1, "只有 600002 共享 2 个非通用概念");
        assert_eq!(first[0].code, "600002");
        assert!(map.get("600004").is_none(), "白酒无共现票");
    }
}
