//! ReviewLhb 投递归档 → 票的时间轴（批 2.6，2026-10-24）。
//!
//! ## 为什么从投递归档读，不读库表
//!
//! `lhb_daily` 表停在 2026-07-12；龙虎榜现在只以 `ReviewLhb` 推送的形式存在
//! （网关直取东财 web 榜）。2026-10-24 探针实测：最近 60 天 35 条决策全部
//! `Delivered`，正文解码后 **35/35 都带「营业部」「席位」「净买」「机构专用」**——
//! 也就是说「谁在买」这一维不缺数据，缺的是消费方。
//!
//! ## 口径
//!
//! - 只读打开 `durable_delivery.sqlite3`（`data/durable_delivery.sqlite3`，cwd=运行根）；
//! - 只认 `state='Delivered'` 的 `ReviewLhb`；正文里 `rendered_content` 是
//!   **UTF-8 字节数组**，不是字符串（照 skill 的坑：先 `bytes()` 再 decode）；
//! - 解析格式（2026-10-22 实测）：
//!
//!   ```text
//!   1. SZ300296 排名净买 1.83亿 | 源披露 1 条
//!      买入席位:
//!        买1 深股通专用 | 2.34亿 | 净=8088.95万
//!   ```
//!
//! - 只给**活跃且分型=情绪型**的票记 `ticket_event(kind='lhb')`：趋势型票看龙虎榜
//!   属于噪音（游资一日游），写进去会把时间轴弄花；
//! - 同日同票同摘要只写一次（重启重放安全）。
//!
//! ## 边界
//!
//! - 归档正文只含**当日净买前五**，覆盖 5 只票/日；不在榜上的活跃票没有事件，这是正确的「无」；
//! - 只记摘要（净买额 + 前两席位），完整席位仍在投递归档里，不复制进主库。

use std::path::Path;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};

use crate::database::{ticket_lifecycle, DatabaseManager};
use crate::monitor::ticket_regime::{self, Regime};

const PUSH_KIND: &str = "ReviewLhb";

#[derive(Debug, Clone, PartialEq)]
pub struct LhbRow {
    pub code: String,
    /// 排名净买（原文，如 `1.83亿`）
    pub net_buy: String,
    /// 前两买入席位（`买1 名称 | 金额`）
    pub top_seats: Vec<String>,
}

#[derive(Debug, Default)]
pub struct LhbReport {
    pub decisions: usize,
    pub parsed_rows: usize,
    pub matched: usize,
    pub written: usize,
    pub errors: Vec<String>,
}

impl LhbReport {
    pub fn log(&self) {
        log::info!(
            "[龙虎榜] decisions={} parsed={} matched={} written={} errors={}",
            self.decisions,
            self.parsed_rows,
            self.matched,
            self.written,
            self.errors.len()
        );
        for error in &self.errors {
            log::warn!("[龙虎榜] {error}");
        }
    }
}

/// 解析一段 ReviewLhb 正文。非个股行（标题/分隔线）忽略。
pub fn parse_render(text: &str) -> Vec<LhbRow> {
    // 2026-10-26：渲染模板从「排名净买」改为「股票名 净买额」，
    // 旧正则完全不匹配 → 龙虎榜自 07-12 起一直解析 0 行。
    // 两种形状都接受：`1. SZ000559 探路者 3.74亿 | ...` 与 `1. SZ000559 排名净买 3.74亿 | ...`
    let header = regex::Regex::new(
        r"^\s*\d+\.\s*(?:SZ|SH)(\d{6})\s+(?:\S+\s+)?(-?\d+(?:\.\d+)?(?:亿|万))\s*\|",
    )
    .unwrap();
    let seat = regex::Regex::new(r"^\s*(买[1-9])\s+(.+?)\s*\|\s*([^|]+)").unwrap();

    let mut rows: Vec<LhbRow> = Vec::new();
    for line in text.lines() {
        if let Some(caps) = header.captures(line) {
            rows.push(LhbRow {
                code: caps[1].to_string(),
                net_buy: caps[2].trim().to_string(),
                top_seats: Vec::new(),
            });
            continue;
        }
        if let Some(caps) = seat.captures(line) {
            if let Some(current) = rows.last_mut() {
                if current.top_seats.len() < 2 {
                    current.top_seats.push(format!(
                        "{} {} {}",
                        caps[1].trim(),
                        caps[2].trim(),
                        caps[3].trim()
                    ));
                }
            }
        }
    }
    // 同一只票可能在正文里出现多次（多笔披露），合并席位、保留第一条净买额。
    let mut merged: Vec<LhbRow> = Vec::new();
    for row in rows {
        match merged.iter_mut().find(|item| item.code == row.code) {
            Some(existing) => {
                for seat in row.top_seats {
                    if existing.top_seats.len() < 2 && !existing.top_seats.contains(&seat) {
                        existing.top_seats.push(seat);
                    }
                }
            }
            None => merged.push(row),
        }
    }
    merged
}

fn decode_rendered(value: &serde_json::Value) -> String {
    match value.get("rendered_content") {
        Some(serde_json::Value::Array(items)) => {
            let bytes: Vec<u8> = items
                .iter()
                .filter_map(|item| item.as_u64().map(|number| number as u8))
                .collect();
            String::from_utf8_lossy(&bytes).to_string()
        }
        Some(serde_json::Value::String(text)) => text.clone(),
        _ => String::new(),
    }
}

/// 从投递归档补记龙虎榜事件。
pub fn sync_from_delivery(
    db: &DatabaseManager,
    delivery_db_path: &Path,
    business_date: &str,
    lookback_days: i64,
) -> Result<LhbReport, String> {
    let delivery = rusqlite::Connection::open_with_flags(
        delivery_db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|error| format!("open delivery db: {error}"))?;

    let since = chrono::NaiveDate::parse_from_str(business_date, "%Y-%m-%d")
        .map_err(|error| format!("business_date 非法 ({business_date}): {error}"))?
        - chrono::Duration::days(lookback_days.max(1));
    let since_text = since.format("%Y-%m-%d").to_string();

    let mut stmt = delivery
        .prepare(
            "SELECT envelope_canonical FROM delivery_decisions \
              WHERE push_kind = ?1 AND state = 'Delivered' AND business_date >= ?2 \
              ORDER BY business_date DESC",
        )
        .map_err(|error| format!("prepare delivery query: {error}"))?;
    let raw_rows = stmt
        .query_map(rusqlite::params![PUSH_KIND, since_text], |row| {
            row.get::<_, Vec<u8>>(0)
        })
        .map_err(|error| format!("query delivery rows: {error}"))?
        .filter_map(Result::ok)
        .collect::<Vec<Vec<u8>>>();

    let mut report = LhbReport {
        decisions: raw_rows.len(),
        ..Default::default()
    };

    let mut by_code: std::collections::HashMap<String, LhbRow> = std::collections::HashMap::new();
    for raw in raw_rows {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&raw) else {
            report.errors.push("envelope 解析失败（跳过）".to_string());
            continue;
        };
        let text = decode_rendered(&value);
        if text.is_empty() {
            continue;
        }
        for row in parse_render(&text) {
            report.parsed_rows += 1;
            by_code.entry(row.code.clone()).or_insert(row);
        }
    }

    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    #[derive(QueryableByName)]
    struct TicketRow {
        #[diesel(sql_type = BigInt)]
        id: i64,
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Text)]
        source: String,
    }
    let tickets: Vec<TicketRow> = diesel::sql_query(
        "SELECT id, code, source FROM ticket_lifecycle WHERE close_date IS NULL ORDER BY code",
    )
    .load(&mut conn)
    .map_err(|error| format!("活跃票查询失败: {error}"))?;

    let event_at = chrono::Local::now().to_rfc3339();
    for ticket in tickets {
        let Some(row) = by_code.get(&ticket.code) else {
            continue;
        };
        let regime = match ticket_regime::resolve(
            &mut conn,
            ticket.id,
            &ticket.code,
            &ticket.source,
            business_date,
        ) {
            Ok(decision) => decision.regime,
            Err(error) => {
                report
                    .errors
                    .push(format!("{} 分型失败: {error}", ticket.code));
                continue;
            }
        };
        if regime != Regime::Emotion {
            continue;
        }
        report.matched += 1;
        let summary = format!(
            "龙虎榜净买 {}｜{}",
            row.net_buy,
            if row.top_seats.is_empty() {
                "席位未解析".to_string()
            } else {
                row.top_seats.join("；")
            }
        );
        if ticket_lifecycle::event_exists_same_day(
            &mut conn,
            ticket.id,
            "lhb",
            business_date,
            &summary,
        )
        .unwrap_or(false)
        {
            continue;
        }
        match ticket_lifecycle::append_event(
            &mut conn,
            ticket.id,
            &event_at,
            "lhb",
            None,
            None,
            Some(&summary),
        ) {
            Ok(()) => report.written += 1,
            Err(error) => report
                .errors
                .push(format!("{} 记事件失败: {error}", ticket.code)),
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "🐉 龙虎榜净买前五（2026-10-22 21:00）\n\
数据源: eastmoney-web | 源日期: 2026-10-22\n\
1. SZ300296 排名净买 1.83亿 | 源披露 1 条\n\
   披露1 TRADE_ID=100429937 | 原因: 连续三个交易日内\n\
   买=6.27亿 卖=4.44亿 净=1.83亿 换手=24.61%\n\
   买入席位:\n\
     买1 深股通专用 | 2.34亿 | 净=8088.95万\n\
     买2 浙商证券股份有限公司杭州五星路证券营业部 | 9902.78万 | 净=9902.35万\n\
     买3 开源证券股份有限公司西安西大街证券营业部 | 8670.15万 | 净=-671.42万\n\
   卖出席位:\n\
     卖1 深股通专用 | 1.53亿 | 净=8088.95万\n\
2. SH600000 排名净买 9000万 | 源披露 1 条\n";

    #[test]
    fn parses_code_net_buy_and_first_two_seats() {
        let rows = parse_render(SAMPLE);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].code, "300296");
        assert_eq!(rows[0].net_buy, "1.83亿");
        assert_eq!(rows[0].top_seats.len(), 2, "只留前两席");
        assert!(rows[0].top_seats[0].contains("深股通专用"));
        assert_eq!(rows[1].code, "600000");
        assert!(rows[1].top_seats.is_empty());
    }

    #[test]
    fn duplicate_code_is_merged() {
        let text = format!("{SAMPLE}1. SZ300296 排名净买 1.83亿 | 源披露 1 条\n");
        let rows = parse_render(&text);
        let same: Vec<&LhbRow> = rows.iter().filter(|row| row.code == "300296").collect();
        assert_eq!(same.len(), 1, "同票多笔披露合并成一行");
    }

    #[test]
    fn empty_text_yields_nothing() {
        assert!(parse_render("").is_empty());
        assert!(parse_render("🐉 龙虎榜净买前五\n数据源: eastmoney-web").is_empty());
    }

    // 2026-10-26：模板改为「代码 股票名 净买额 | ...」，旧正则 0 行。
    const SAMPLE_V2: &str = "🐉 龙虎榜净买前五（2026-10-26 21:00）\n\
数据源: eastmoney-web | 源日期: 2026-10-26\n\
1. SZ000559 探路者 3.74亿 | 净买额 2 笔\n\
   买1 深股通专用 | 2.51亿 | 净=1.26亿\n\
   买2 机构专用 | 9467.28万 | 净=9467.28万\n";

    #[test]
    fn parses_current_template_with_stock_name() {
        let rows = parse_render(SAMPLE_V2);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].code, "000559");
        assert_eq!(rows[0].net_buy, "3.74亿");
        assert_eq!(rows[0].top_seats.len(), 2);
    }
}
