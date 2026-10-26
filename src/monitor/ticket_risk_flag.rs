//! 排雷标注（批 2.7 / 拓 6，2026-10-23）。
//!
//! ## 定位
//!
//! 财报与公告在短线里**不进选票打分**（季度低频信息，打分只会系统性错过最强票），
//! 但 T+1 下买入当日不能纠错，暴雷是唯一不能自愈的风险 —— 所以只做**标注，不挡买入**。
//!
//! ## 做法
//!
//! 每日扫 `stock_f10_profile` 的四类原文，命中关键词就写
//! `ticket_event(kind='risk_flag', summary='风险提示：退市…；减持…')`，
//! 并抽命中处前后 40 字作为证据片段（`render_plan` 渲染「⚠ 有雷」时用）。
//!
//! 关键词：退市 / 立案 / 预亏 / 商誉减值 / 减持 / 解禁。
//!
//! ## 边界（必须写进汇报）
//!
//! - **关键词会误报**：F10 的「最新提示」是模板化风险提示，几乎每只票都有
//!   「本公司不存在退市风险」这类句子。所以这条只做「值得看一眼」的标注，
//!   不做买卖决定；上线后按方案用两周调词表。
//! - F10 是快照，不是公告流；真正的权威源是 ExternalV1 `Announcements`
//!   （Mac 侧尚未接线，见方案 2.7 补充）。
//! - 只扫活跃票（持仓 + ticket），不做全市场。

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};

use crate::database::{ticket_lifecycle, DatabaseManager};

/// 风险关键词（顺序即渲染顺序：越靠前越严重）。
pub const RISK_KEYWORDS: &[&str] = &["退市", "立案", "预亏", "商誉减值", "减持", "解禁"];

/// 证据片段半径（字符数）。
const SNIPPET_RADIUS: usize = 20;

#[derive(Debug, Clone)]
pub struct RiskHit {
    pub ticket_id: i64,
    pub code: String,
    pub name: String,
    pub keyword: String,
    /// 命中处前后各 20 字，用于人工核对
    pub snippet: String,
}

impl RiskHit {
    pub fn render(&self) -> String {
        format!(
            "{} {}〔{}〕{}",
            self.code, self.name, self.keyword, self.snippet
        )
    }
}

#[derive(Debug, Default)]
pub struct RiskReport {
    pub scanned: usize,
    pub hits: Vec<RiskHit>,
    pub written: usize,
    pub errors: Vec<String>,
}

impl RiskReport {
    pub fn log(&self) {
        log::info!(
            "[排雷] scanned={} hits={} written={} errors={}",
            self.scanned,
            self.hits.len(),
            self.written,
            self.errors.len()
        );
        for error in &self.errors {
            log::warn!("[排雷] {error}");
        }
    }

    /// 推送正文：一条消息装所有票，按 code 分组。
    pub fn render(&self, as_of: &str) -> Option<String> {
        if self.hits.is_empty() {
            return None;
        }
        let mut out = format!("【{as_of} 排雷标注】\n");
        out.push_str("只标注、不挡买入；T+1 不能当日纠错，看到就多看一眼。\n");
        let mut last_code = String::new();
        for hit in &self.hits {
            if hit.code != last_code {
                out.push('\n');
                last_code = hit.code.clone();
            }
            out.push_str(&format!("{}\n", hit.render()));
        }
        Some(out)
    }
}

/// 从原文里抽命中处前后 `SNIPPET_RADIUS` 字（按字符，不按字节 —— 中文会被切碎）。
fn snippet_around(content: &str, needle: &str) -> Option<String> {
    let index = content.find(needle)?;
    let before: String = content[..index]
        .chars()
        .rev()
        .take(SNIPPET_RADIUS)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let after: String = content[index + needle.len()..]
        .chars()
        .take(SNIPPET_RADIUS)
        .collect();
    let mut text = format!("{before}{needle}{after}");
    text = text.replace(['\n', '\r', '\t'], " ");
    Some(text.trim().to_string())
}

#[derive(QueryableByName)]
struct ProfileRow {
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    content: String,
}

/// 扫一遍活跃票的 F10 原文，写 `risk_flag` 事件。
pub fn scan(db: &DatabaseManager, as_of: &str) -> Result<RiskReport, String> {
    #[derive(QueryableByName)]
    struct TicketRow {
        #[diesel(sql_type = BigInt)]
        id: i64,
        #[diesel(sql_type = Text)]
        code: String,
    }

    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    let tickets: Vec<TicketRow> = diesel::sql_query(
        "SELECT id, code FROM ticket_lifecycle WHERE close_date IS NULL ORDER BY code",
    )
    .load(&mut conn)
    .map_err(|error| format!("活跃票查询失败: {error}"))?;

    let mut report = RiskReport {
        scanned: tickets.len(),
        ..Default::default()
    };
    if tickets.is_empty() {
        return Ok(report);
    }

    // F10 只有千级行，全量取回在 Rust 侧按 code 过滤，避免动态 IN 绑定。
    let profiles: Vec<ProfileRow> =
        diesel::sql_query("SELECT code, content FROM stock_f10_profile")
            .load(&mut conn)
            .map_err(|error| format!("F10 原文查询失败: {error}"))?;
    let by_code: std::collections::HashMap<&str, Vec<&str>> = {
        let mut map: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
        for row in &profiles {
            map.entry(row.code.as_str())
                .or_default()
                .push(row.content.as_str());
        }
        map
    };

    let event_at = chrono::Local::now().to_rfc3339();
    for ticket in tickets {
        let Some(contents) = by_code.get(ticket.code.as_str()) else {
            continue;
        };
        let mut hits: Vec<RiskHit> = Vec::new();
        for keyword in RISK_KEYWORDS {
            let mut found: Option<String> = None;
            for content in contents {
                if let Some(snippet) = snippet_around(content, keyword) {
                    found = Some(snippet);
                    break;
                }
            }
            if let Some(snippet) = found {
                hits.push(RiskHit {
                    ticket_id: ticket.id,
                    code: ticket.code.clone(),
                    // 2026-10-26：原来直接写 code，推送里就只剩数字。
                    name: crate::monitor::buy_zone::resolve_name(&mut conn, &ticket.code)
                        .unwrap_or_else(|| ticket.code.clone()),
                    keyword: (*keyword).to_string(),
                    snippet,
                });
            }
        }
        if hits.is_empty() {
            continue;
        }
        // 摘要只带关键词（片段太长会撑爆事件表），片段留给推送正文。
        let summary = format!(
            "风险提示：{}",
            hits.iter()
                .map(|hit| hit.keyword.clone())
                .collect::<Vec<_>>()
                .join("、")
        );
        let already = ticket_lifecycle::event_exists_same_day(
            &mut conn,
            ticket.id,
            "risk_flag",
            as_of,
            &summary,
        )
        .unwrap_or(false);
        if !already {
            match ticket_lifecycle::append_event(
                &mut conn,
                ticket.id,
                &event_at,
                "risk_flag",
                None,
                None,
                Some(&summary),
            ) {
                Ok(()) => report.written += 1,
                Err(error) => {
                    report
                        .errors
                        .push(format!("{} 记事件失败: {error}", ticket.code));
                    continue;
                }
            }
        }
        report.hits.extend(hits);
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_keeps_context_and_squashes_newlines() {
        let content = "公司公告：\n本公司不存在退市风险，经营正常。";
        let snippet = snippet_around(content, "退市").expect("hit");
        assert!(snippet.contains("退市"));
        assert!(snippet.contains("不存在"));
        assert!(!snippet.contains('\n'));
    }

    #[test]
    fn snippet_missing_keyword_is_none() {
        assert!(snippet_around("经营正常", "立案").is_none());
    }

    #[test]
    fn keywords_cover_documented_set() {
        for keyword in ["退市", "立案", "预亏", "商誉减值", "减持", "解禁"] {
            assert!(RISK_KEYWORDS.contains(&keyword), "缺关键词 {keyword}");
        }
    }
}
