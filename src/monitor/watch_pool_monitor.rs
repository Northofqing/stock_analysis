//! 观察池定向监控（2026-10-18）。
//!
//! ## 为什么现在才做
//!
//! 技术条件一直齐备：`realtime_quotes_async` 存在、观察池 `ticket_lifecycle` 存在、
//! 主循环本来就是 30 秒一轮（`main.rs` 的 `sleep(Duration::from_secs(30))`）。
//! 之前把它列为"没做"只是因为没排优先级，不是做不了。
//!
//! ## 三条异动规则（任一命中）
//!
//! 1. **日内涨跌幅** 绝对值 >= 3%
//! 2. **相对上次观测价** 变动 >= 1%（捕捉盘中急拉急跌）
//! 3. **换手异常** —— 暂不实现，需要成交量字段，当前行情批次没带
//!
//! ## 边界
//!
//! - **只扫描**，不做决策、不改 `stage`、不写库
//! - 去重由调用方负责（同一只票反复触发同一条规则时不该反复推送）
//! - 行情批次不完整（`Partial`）时照常返回能拿到的部分，但调用方应记录缺口
//! - 停牌票拿不到行情，自动不出现在结果里，这是正确行为不是 bug

use std::collections::{HashMap, HashSet};

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};

use crate::data_gateway::GatewayBatch;
use crate::database::DatabaseManager;

/// 日内涨跌幅阈值（%）
pub const DAY_CHANGE_THRESHOLD: f64 = 3.0;
/// 相对上次观测价的变动阈值（小数，0.01 = 1%）
pub const TICK_CHANGE_THRESHOLD: f64 = 0.01;
/// 新闻催化回看窗口（秒）。10 分钟足够覆盖盘中快讯，
/// 又不会把早上的新闻反复报出来。
pub const NEWS_LOOKBACK_SECS: i64 = 600;
/// 同板块关联票的涨幅阈值（%）。
///
/// 设 5% 而不是 3%：板块内每天都有一堆 3% 的票，报出来等于噪音；
/// 5% 以上才算「板块里有票在独立走强」，对判断板块是否还在有参考价值。
pub const PEER_CHANGE_THRESHOLD: f64 = 5.0;
/// 每轮最多查多少只关联票行情（保护行情接口，也避免日志爆炸）。
/// 同板块关联票单次最多拉取的候选数。
///
/// 200 会直接触发 provider 拒绝：Tencent 行情接口对单次批量有上限，
/// 超限返回 internal，导致每 30 秒刷一条 warn 且规则整轮跳过。
/// 50 是实测可用的批量；超出部分下一轮继续（`load_peer_alerts` 按板块分组，
/// 每个板块只报最强一只，50 只足以覆盖观察池 16 只票的同板块邻居）。
const PEER_QUERY_LIMIT: usize = 50;

#[derive(Debug, Clone)]
pub struct WatchAlert {
    pub code: String,
    pub name: String,
    pub price: f64,
    pub change_percent: f64,
    /// 触发了哪几条规则，用"；"分隔
    pub reason: String,
}

impl WatchAlert {
    pub fn render(&self) -> String {
        format!(
            "{} {} 现价 {:.2}（{:+.2}%）— {}",
            self.code, self.name, self.price, self.change_percent, self.reason
        )
    }
}

/// 监控标的：观察池全部 active ticket + 当前持仓。
fn watch_codes(conn: &mut SqliteConnection) -> Result<Vec<String>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        code: String,
    }
    let mut set: HashSet<String> = HashSet::new();
    let rows: Vec<Row> =
        diesel::sql_query("SELECT code FROM ticket_lifecycle WHERE close_date IS NULL")
            .load(conn)
            .map_err(|e| e.to_string())?;
    for row in rows {
        set.insert(row.code);
    }
    for position in crate::portfolio::load_positions().unwrap_or_default() {
        set.insert(position.code);
    }
    let mut codes: Vec<String> = set.into_iter().collect();
    codes.sort();
    Ok(codes)
}

/// 行情请求要求「非空且唯一」，而 ticket/板块/概念三个来源可能有重复或空白。
/// 在发请求前统一清理，避免整批被 invalid_request 拒绝。
fn normalized_unique_codes(codes: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    codes
        .into_iter()
        .filter_map(|code| {
            let code = code.trim();
            if code.is_empty() || !seen.insert(code.to_owned()) {
                None
            } else {
                Some(code.to_owned())
            }
        })
        .collect()
}

/// 扫描一轮。返回本轮触发的异动。
///
/// `last_prices` 是上一轮的观测价（调用方维护），用于判断盘中急变。
/// 首轮传空 map 即可 —— 此时只有"日内涨跌幅"规则能触发。
pub async fn scan(
    db: &DatabaseManager,
    last_prices: &HashMap<String, f64>,
) -> Result<Vec<WatchAlert>, String> {
    let codes = {
        let mut conn = db.get_conn().map_err(|e| e.to_string())?;
        watch_codes(&mut conn)?
    };
    let codes = normalized_unique_codes(codes);
    if codes.is_empty() {
        return Ok(Vec::new());
    }

    // P3.1: 新闻催化。观察池的票如果 10 分钟内出现了新新闻/公告，
    // 即使价格没动也要报——这正是「新闻驱动」和「纯价格异动」的区别。
    // 已通知过的新闻用 news_dedup 记住，避免 30 秒一轮反复刷屏。
    let news_hits = match load_fresh_news(db, &codes) {
        Ok(hits) => hits,
        Err(error) => {
            log::warn!("[观察池] 新闻查询失败, 本轮只跑价格规则: {}", error);
            Vec::new()
        }
    };

    // P3.3: 同板块关联票异动。观察池的票今天可能不动，但它所在板块里
    // 已经有票在走强 —— 这是「板块还在不在」的直接证据，值得报。
    let peer_alerts = match load_peer_alerts(db, &codes).await {
        Ok(alerts) => alerts,
        Err(error) => {
            log::warn!("[观察池] 关联票查询失败, 本轮跳过该规则: {error}");
            HashMap::new()
        }
    };

    let bridge = crate::data_gateway::grpc_source::bridge_for("RealtimeQuotes")
        .map_err(|error| error.to_string())?;
    let quotes = match bridge.realtime_quotes_async(&codes).await {
        Ok(GatewayBatch::Available { records, .. }) => records,
        Ok(GatewayBatch::VerifiedEmpty(_)) => return Ok(Vec::new()),
        // 整批被「当日无最新成交价」拒绝（停牌 / 未开盘 / 一字板封死）。
        // 这条是业务态不是故障，二分隔离出坏票，好票继续参与异动判断；
        // 坏票本轮直接丢，不造价格（BR-222）。
        Err(error) if error.reason_code() == "no_last_traded_price" => {
            let isolated = bridge.realtime_quotes_with_isolation(&codes).await;
            log::warn!(
                "[观察池] RealtimeQuotes 被 no_last_traded_price 拒绝, 二分隔离后 \
                 取到 {} 只 / 隔离 {} 只 / 请求 {} 次{}",
                isolated.quotes.len(),
                isolated.isolated.len(),
                isolated.requests,
                if isolated.isolated.is_empty() {
                    String::new()
                } else {
                    format!("（隔离: {}）", isolated.isolated.join(","))
                }
            );
            if isolated.quotes.is_empty() {
                return Ok(Vec::new());
            }
            isolated.quotes
        }
        Err(error) => return Err(format!("行情拉取失败: {error}")),
    };

    let mut news_by_code: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    for hit in news_hits {
        news_by_code
            .entry(hit.code.clone())
            .or_default()
            .push((hit.id, hit.title));
    }

    let mut alerts: Vec<WatchAlert> = Vec::new();
    let mut notified_news: Vec<i64> = Vec::new();
    for quote in quotes {
        let mut reasons: Vec<String> = Vec::new();

        if quote.change_percent.abs() >= DAY_CHANGE_THRESHOLD {
            reasons.push(format!("日内 {:+.2}%", quote.change_percent));
        }

        if let Some(previous) = last_prices.get(&quote.code) {
            if *previous > 0.0 {
                let tick = (quote.price - *previous) / *previous;
                if tick.abs() >= TICK_CHANGE_THRESHOLD {
                    reasons.push(format!("较上次 {:+.2}%", tick * 100.0));
                }
            }
        }

        if let Some(items) = news_by_code.get(&quote.code) {
            if let Some((_, title)) = items.first() {
                let trimmed: String = title.chars().take(40).collect();
                if items.len() == 1 {
                    reasons.push(format!("新闻：{trimmed}"));
                } else {
                    reasons.push(format!("新闻 {} 条：{trimmed}", items.len()));
                }
                for (id, _) in items {
                    notified_news.push(*id);
                }
            }
        }

        if let Some(peers) = peer_alerts.get(&quote.code) {
            for peer in peers {
                reasons.push(format!(
                    "{}「{}」{} {} {:+.2}%",
                    peer.source, peer.board_name, peer.code, peer.name, peer.change_pct
                ));
            }
        }

        if reasons.is_empty() {
            continue;
        }
        alerts.push(WatchAlert {
            code: quote.code,
            name: quote.name,
            price: quote.price,
            change_percent: quote.change_percent,
            reason: reasons.join("；"),
        });
    }

    if !notified_news.is_empty() {
        if let Ok(mut conn) = db.get_conn() {
            mark_news_notified(&mut conn, &notified_news);
        }
    }

    alerts.sort_by(|a, b| {
        b.change_percent
            .abs()
            .partial_cmp(&a.change_percent.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(alerts)
}

/// 同板块里异动的一只票。
struct PeerHit {
    code: String,
    name: String,
    price: f64,
    change_pct: f64,
    /// 「同板块」/「同概念」——两个来源的强弱不同，文案里必须能分开
    source: &'static str,
    board_name: String,
}

/// 同板块关联票异动：观察池的票属于哪些板块，这些板块里今天还有谁在走强。
///
/// 为什么是「同板块」而不是「关联公司」：`chain_relation_candidate` 存的
/// 是产业环节（钢材/原料药），`object_code` 全部是 NULL，盯不了股价。
/// 目前唯一带可交易代码的关联关系是「同板块」——`board_member` 有八百多条成分。
///
/// 每个板块只保留涨幅最高的一只，避免同一板块刷出十几条。
async fn load_peer_alerts(
    db: &DatabaseManager,
    codes: &[String],
) -> Result<HashMap<String, Vec<PeerHit>>, String> {
    #[derive(QueryableByName)]
    struct MemberRow {
        #[diesel(sql_type = Text)]
        board_code: String,
        #[diesel(sql_type = Text)]
        board_name: String,
        #[diesel(sql_type = Text)]
        instrument_code: String,
    }

    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    // board_member 只有千级行，全量取回在 Rust 侧过滤，避免动态 IN 绑定。
    let members: Vec<MemberRow> =
        diesel::sql_query("SELECT board_code, board_name, instrument_code FROM board_member")
            .load(&mut conn)
            .map_err(|error| format!("板块成分查询失败: {error}"))?;
    drop(conn);

    let watched: HashSet<&str> = codes.iter().map(String::as_str).collect();
    let mut boards_of: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
    let mut board_members: HashMap<&str, Vec<&str>> = HashMap::new();
    for row in &members {
        board_members
            .entry(row.board_code.as_str())
            .or_default()
            .push(row.instrument_code.as_str());
        if watched.contains(row.instrument_code.as_str()) {
            boards_of
                .entry(row.instrument_code.as_str())
                .or_default()
                .push((row.board_code.as_str(), row.board_name.as_str()));
        }
    }

    let mut candidates: Vec<String> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for boards in boards_of.values() {
        for (board_code, _) in boards {
            if let Some(list) = board_members.get(board_code) {
                for code in list {
                    if watched.contains(code) || !seen.insert(code) {
                        continue;
                    }
                    candidates.push((*code).to_owned());
                    if candidates.len() >= PEER_QUERY_LIMIT {
                        break;
                    }
                }
            }
            if candidates.len() >= PEER_QUERY_LIMIT {
                break;
            }
        }
        if candidates.len() >= PEER_QUERY_LIMIT {
            break;
        }
    }
    // 批 2.5（拓 2.3，2026-10-24）：概念共现作为第二来源。
    // `stock_concepts` 覆盖 1265 只，比 `board_member` 的采集范围广；
    // 一次装载（ConceptIndex）后按票分组，不再逐票全表扫。
    let concept_map: HashMap<String, Vec<crate::monitor::concept_peers::Peer>> =
        match crate::monitor::concept_peers::ConceptIndex::load(db) {
            Ok(index) => index.peers_many(codes, crate::monitor::concept_peers::DEFAULT_MIN_SHARED),
            Err(error) => {
                log::warn!("[观察池] 概念装载失败, 本轮只用板块: {error}");
                HashMap::new()
            }
        };
    let mut candidate_set: HashSet<String> = candidates.iter().cloned().collect();
    'concept_candidates: for peers in concept_map.values() {
        for peer in peers {
            if candidates.len() >= PEER_QUERY_LIMIT {
                break 'concept_candidates;
            }
            if candidate_set.insert(peer.code.clone()) {
                candidates.push(peer.code.clone());
            }
        }
    }

    if candidates.is_empty() {
        return Ok(HashMap::new());
    }
    let candidates = normalized_unique_codes(candidates);
    if candidates.is_empty() {
        return Ok(HashMap::new());
    }

    let bridge = crate::data_gateway::grpc_source::bridge_for("RealtimeQuotes")
        .map_err(|error| error.to_string())?;
    let quotes = match bridge.realtime_quotes_async(&candidates).await {
        Ok(GatewayBatch::Available { records, .. }) => records,
        Ok(GatewayBatch::VerifiedEmpty(_)) => return Ok(HashMap::new()),
        Err(error) => return Err(format!("关联票行情拉取失败: {error}")),
    };
    let by_code: HashMap<&str, &crate::data_gateway::RealtimeMarketQuote> = quotes
        .iter()
        .map(|quote| (quote.code.as_str(), quote))
        .collect();

    // subject -> board_code -> 最强的一只 peer
    let mut best: HashMap<String, HashMap<String, PeerHit>> = HashMap::new();
    for (subject, boards) in &boards_of {
        for (board_code, board_name) in boards {
            let Some(list) = board_members.get(board_code) else {
                continue;
            };
            for peer in list {
                if watched.contains(peer) {
                    continue;
                }
                let Some(quote) = by_code.get(peer) else {
                    continue;
                };
                if quote.change_percent < PEER_CHANGE_THRESHOLD {
                    continue;
                }
                let slot = best
                    .entry((*subject).to_owned())
                    .or_default()
                    .entry((*board_code).to_owned())
                    .or_insert_with(|| PeerHit {
                        code: quote.code.clone(),
                        name: quote.name.clone(),
                        price: quote.price,
                        change_pct: quote.change_percent,
                        source: "同板块",
                        board_name: (*board_name).to_owned(),
                    });
                if quote.change_percent > slot.change_pct {
                    slot.code = quote.code.clone();
                    slot.name = quote.name.clone();
                    slot.price = quote.price;
                    slot.change_pct = quote.change_percent;
                }
            }
        }
    }

    // 同概念只报最强一只（概念组多，逐组报会刷屏）。
    for (subject, peers) in &concept_map {
        for peer in peers {
            let Some(quote) = by_code.get(peer.code.as_str()) else {
                continue;
            };
            if quote.change_percent < PEER_CHANGE_THRESHOLD {
                continue;
            }
            let slot = best
                .entry(subject.clone())
                .or_default()
                .entry("concept".to_string())
                .or_insert_with(|| PeerHit {
                    code: quote.code.clone(),
                    name: quote.name.clone(),
                    price: quote.price,
                    change_pct: quote.change_percent,
                    source: "同概念",
                    board_name: peer.shared.join("、"),
                });
            if quote.change_percent > slot.change_pct {
                slot.code = quote.code.clone();
                slot.name = quote.name.clone();
                slot.price = quote.price;
                slot.change_pct = quote.change_percent;
                slot.board_name = peer.shared.join("、");
            }
        }
    }

    Ok(best
        .into_iter()
        .map(|(subject, boards)| (subject, boards.into_values().collect()))
        .collect())
}

/// 一条待报的新闻。
struct FreshNews {
    id: i64,
    code: String,
    title: String,
}

/// 最近 10 分钟、还没通知过的、属于观察池的新闻。
///
/// 先按时间窗口取一批（单次 bind，避免动态 IN 的类型问题），
/// 再在 Rust 里按观察池过滤——观察池通常只有几只到几十只票，
/// 这批上限 200 条足够，不会漏。
fn load_fresh_news(db: &DatabaseManager, codes: &[String]) -> Result<Vec<FreshNews>, String> {
    if codes.is_empty() {
        return Ok(Vec::new());
    }
    let cutoff = chrono::Utc::now().timestamp() - NEWS_LOOKBACK_SECS;
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;

    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = BigInt)]
        id: i64,
        #[diesel(sql_type = diesel::sql_types::Nullable<Text>)]
        code: Option<String>,
        #[diesel(sql_type = Text)]
        title: String,
    }

    // code 为空的宏观新闻交给 NewsFlash 链路，不在这里报。
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT n.id AS id, n.code AS code, n.title AS title
           FROM news_items n
          WHERE n.code IS NOT NULL
            AND n.fetched_at > ?
            AND NOT EXISTS (
                  SELECT 1 FROM news_dedup d WHERE d.key = 'watch_news:' || n.id
                )
          ORDER BY n.published_at DESC
          LIMIT 200",
    )
    .bind::<BigInt, _>(cutoff)
    .load::<Row>(&mut conn)
    .map_err(|error| format!("新闻查询失败: {error}"))?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let code = row.code?;
            if !codes.iter().any(|known| known == &code) {
                return None;
            }
            Some(FreshNews {
                id: row.id,
                code,
                title: row.title,
            })
        })
        .collect())
}

/// 记下已通知的新闻，避免下一轮重复。
fn mark_news_notified(conn: &mut SqliteConnection, ids: &[i64]) {
    for id in ids {
        let key = format!("watch_news:{id}");
        let _ = diesel::sql_query("INSERT OR IGNORE INTO news_dedup (key) VALUES (?)")
            .bind::<Text, _>(key)
            .execute(conn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_are_documented_values() {
        assert!((DAY_CHANGE_THRESHOLD - 3.0).abs() < f64::EPSILON);
        assert!((TICK_CHANGE_THRESHOLD - 0.01).abs() < f64::EPSILON);
    }

    #[test]
    fn tick_detection_math() {
        let previous = 10.0_f64;
        let now = 10.15_f64;
        let tick = (now - previous) / previous;
        assert!(tick >= TICK_CHANGE_THRESHOLD);
        // 0.5% 变动不该触发
        let small = (10.05_f64 - previous) / previous;
        assert!(small < TICK_CHANGE_THRESHOLD);
    }

    #[test]
    fn alert_render_includes_code_and_name() {
        let alert = WatchAlert {
            code: "600703".to_string(),
            name: "三安光电".to_string(),
            price: 10.18,
            change_percent: -3.5,
            reason: "日内 -3.50%".to_string(),
        };
        let text = alert.render();
        assert!(text.contains("600703"));
        assert!(text.contains("三安光电"));
        assert!(text.contains("-3.50%"));
    }

    #[test]
    fn normalized_codes_trim_and_dedupe() {
        let got = normalized_unique_codes(vec![
            "001216".to_string(),
            " 001216 ".to_string(),
            "".to_string(),
            "  ".to_string(),
            "600396".to_string(),
            "600396".to_string(),
        ]);
        assert_eq!(got, vec!["001216".to_string(), "600396".to_string()]);
    }
}
