//! 持仓财报分析（Day 1）：季报单季拆分 → 指标 → 具体仓位建议。
//!
//! 覆盖范围（operator 2026-10-25 拍板）：持仓 ∪ 虚拟仓位。
//! 推送形态：披露当晚 20:30 一条「财报速览」，22:30 补一轮；次日 08:50 不在这里做。
//!
//! 数据：`CompanyDataGateway::financial_statements`（income / cash_flow）。
//! 利润表与现金流是**年初至今累计**，单季 = 本期累计 − 同年上一期累计。
//! 资产负债表当前源不可用（002131/600396 均 invalid evidence），所以「应收增速」这一项
//! 明确标 `balance_unavailable`，不计算、不补 0。
//!
//! 建议阈值是**预注册**的（2026-10-25 写死，4~6 周样本外再校准）：
//! - 清仓 100%：单季营收同比 < −30% 且 单季归母净利同比 < −50%
//! - 减半 50%：单季归母净利同比 < −30%，或（经营现金流为负 且 净利为正 且 归母净利同比 < 0）
//! - 减 30%：经营现金流/净利 < 0 且 非经常项占归母净利 > 50%
//! - 加仓 20%：单季营收同比 > +20% 且 归母净利同比 > +30% 且 经营现金流/净利 > 0.8
//! - 其余：持有 0%

use crate::data_gateway::company::CompanyDataGateway;
use crate::data_gateway::GatewayBatch;
use crate::market_domain::StatementKind;
use crate::database::DatabaseManager;
use chrono::NaiveDate;
use diesel::sql_types::{Double, Integer, Nullable, Text};
use diesel::RunQueryDsl as _;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};

pub const TRIGGER_ENV_UNUSED: &str = "HOLDING_EARNINGS_TRIGGER_UNUSED";

pub fn ensure_schema(conn: &mut diesel::SqliteConnection) -> Result<(), String> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS financial_statement (\
           code TEXT NOT NULL,\
           kind TEXT NOT NULL,\
           report_period TEXT NOT NULL,\
           announced_on TEXT,\
           lines_json TEXT NOT NULL,\
           provider TEXT,\
           batch_id TEXT,\
           captured_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,\
           PRIMARY KEY (code, kind, report_period))",
    )
    .execute(conn)
    .map_err(|error| format!("financial_statement 建表失败: {error}"))?;
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS holding_earnings_report (\
           code TEXT NOT NULL,\
           report_period TEXT NOT NULL,\
           announced_on TEXT,\
           advice TEXT NOT NULL,\
           ratio_pct INTEGER NOT NULL,\
           reason TEXT NOT NULL,\
           rendered TEXT NOT NULL,\
           pushed_at TEXT,\
           PRIMARY KEY (code, report_period))",
    )
    .execute(conn)
    .map_err(|error| format!("holding_earnings_report 建表失败: {error}"))?;
    Ok(())
}

/// 覆盖范围：最新持仓快照 ∪ 虚拟仓位（paper book 的 lots）。
pub fn covered_codes(conn: &mut diesel::SqliteConnection) -> Vec<String> {
    #[derive(diesel::QueryableByName)]
    struct CodeRow {
        #[diesel(sql_type = Text)]
        code: String,
    }
    let mut codes: Vec<String> = diesel::sql_query(
        "SELECT DISTINCT item.code AS code FROM user_position_snapshot_item item \
          JOIN user_position_snapshot snap ON snap.snapshot_id = item.snapshot_id \
         WHERE snap.snapshot_id = (SELECT snapshot_id FROM user_position_snapshot ORDER BY effective_at DESC LIMIT 1)",
    )
    .load::<CodeRow>(conn)
    .map(|rows| rows.into_iter().map(|row| row.code).collect())
    .unwrap_or_default();

    // 虚拟仓位：paper_ledger_head.projection_bytes 里 lots[].code
    #[derive(diesel::QueryableByName)]
    struct ProjectionRow {
        #[diesel(sql_type = Text)]
        projection_bytes: String,
    }
    if let Ok(row) = diesel::sql_query(
        "SELECT projection_bytes FROM paper_ledger_head ORDER BY rowid DESC LIMIT 1",
    )
    .get_result::<ProjectionRow>(conn)
    {
        if let Ok(value) = serde_json::from_str::<Value>(&row.projection_bytes) {
            if let Some(lots) = value.get("lots").and_then(|lots| lots.as_array()) {
                for lot in lots {
                    if let Some(code) = lot.get("code").and_then(|code| code.as_str()) {
                        let code = code.to_string();
                        if !codes.contains(&code) {
                            codes.push(code);
                        }
                    }
                }
            }
        }
    }
    codes.sort();
    codes
}

fn line_map(statement: &crate::market_domain::FinancialStatement) -> Map<String, Value> {
    let mut map = Map::new();
    for line in &statement.lines {
        let key = line.key.as_str().to_string();
        let value = line.value.map(|value| value.get()).unwrap_or(0.0);
        map.insert(key, json!(value));
    }
    map
}

/// 拉取并落库（income + cash_flow）。返回写入行数与失败明细。
pub async fn sync(db: &DatabaseManager, codes: &[String]) -> Result<(usize, Vec<String>), String> {
    let gateway = CompanyDataGateway::new();
    let mut written = 0usize;
    let mut errors = Vec::new();
    for code in codes {
        for kind in [StatementKind::Income, StatementKind::CashFlow] {
            let batch = match gateway.financial_statements(std::slice::from_ref(code), kind).await {
                Ok(GatewayBatch::Available { records, .. }) => records,
                Ok(GatewayBatch::VerifiedEmpty(_)) => Vec::new(),
                Err(error) => {
                    errors.push(format!("{code}/{:?}: {error}", kind));
                    continue;
                }
            };
            for statement in batch {
                let period = statement.report_period.to_string();
                let lines = serde_json::to_string(&Value::Object(line_map(&statement)))
                    .map_err(|error| error.to_string())?;
                let evidence = &statement.evidence;
                let mut conn = db.get_conn().map_err(|error| error.to_string())?;
                let kind_text = format!("{:?}", kind);
                let announced = statement.announced_on.as_ref().map(|d| d.to_string());
                diesel::sql_query(
                    "INSERT INTO financial_statement \
                     (code, kind, report_period, announced_on, lines_json, provider, batch_id) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                     ON CONFLICT(code, kind, report_period) DO UPDATE SET \
                       announced_on=excluded.announced_on, lines_json=excluded.lines_json, \
                       provider=excluded.provider, batch_id=excluded.batch_id, \
                       captured_at=CURRENT_TIMESTAMP",
                )
                .bind::<Text, _>(code.clone())
                .bind::<Text, _>(kind_text)
                .bind::<Text, _>(period)
                .bind::<Nullable<Text>, _>(announced)
                .bind::<Text, _>(lines)
                .bind::<Nullable<Text>, _>(Some(format!("{:?}", evidence.provider())))
                .bind::<Nullable<Text>, _>(Some(evidence.batch_id().to_string()))
                .execute(&mut conn)
                .map_err(|error| format!("{code} 财报写库失败: {error}"))?;
                written += 1;
            }
        }
    }
    Ok((written, errors))
}

/// 最新一期利润表报告期（没有返回 None）。
pub fn latest_income_period(conn: &mut diesel::SqliteConnection, code: &str) -> Option<String> {
    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        report_period: String,
    }
    diesel::sql_query(
        "SELECT report_period FROM financial_statement WHERE code = ?1 AND kind = 'Income' ORDER BY report_period DESC LIMIT 1",
    )
    .bind::<Text, _>(code)
    .get_result::<Row>(conn)
    .ok()
    .map(|row| row.report_period)
}

/// 公告日（利润表口径）。
pub fn announced_on(conn: &mut diesel::SqliteConnection, code: &str, period: &str) -> Option<String> {
    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = Nullable<Text>)]
        announced_on: Option<String>,
    }
    diesel::sql_query(
        "SELECT announced_on FROM financial_statement WHERE code = ?1 AND kind = 'Income' AND report_period = ?2",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(period)
    .get_result::<Row>(conn)
    .ok()
    .and_then(|row| row.announced_on)
}

/// 虚拟仓位代码（paper book 的 lots），用于推送打〔虚拟〕标签。
pub fn virtual_codes(conn: &mut diesel::SqliteConnection) -> Vec<String> {
    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        projection_bytes: String,
    }
    let mut codes = Vec::new();
    if let Ok(row) = diesel::sql_query(
        "SELECT projection_bytes FROM paper_ledger_head ORDER BY rowid DESC LIMIT 1",
    )
    .get_result::<Row>(conn)
    {
        if let Ok(value) = serde_json::from_str::<Value>(&row.projection_bytes) {
            if let Some(lots) = value.get("lots").and_then(|lots| lots.as_array()) {
                for lot in lots {
                    if let Some(code) = lot.get("code").and_then(|code| code.as_str()) {
                        codes.push(code.to_string());
                    }
                }
            }
        }
    }
    codes
}

#[derive(Debug, Clone, Default)]
pub struct Metrics {
    pub code: String,
    pub report_period: String,
    pub announced_on: Option<String>,
    pub rev_single: Option<f64>,
    pub rev_yoy: Option<f64>,
    pub np_single: Option<f64>,
    pub np_yoy: Option<f64>,
    pub gm_single: Option<f64>,
    pub gm_qoq: Option<f64>,
    pub ocf_single: Option<f64>,
    pub ocf_ratio: Option<f64>,
    pub nonrec_ratio: Option<f64>,
    pub balance_unavailable: bool,
}

#[derive(Debug, Clone)]
pub struct Advice {
    pub action: &'static str,
    pub ratio_pct: i32,
    pub reason: String,
}

fn load_periods(conn: &mut diesel::SqliteConnection, code: &str, kind: &str) -> BTreeMap<String, Map<String, Value>> {
    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        report_period: String,
        #[diesel(sql_type = Text)]
        lines_json: String,
    }
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT report_period, lines_json FROM financial_statement WHERE code = ?1 AND kind = ?2",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(kind)
    .load(conn)
    .unwrap_or_default();
    let mut map = BTreeMap::new();
    for row in rows {
        if let Ok(Value::Object(lines)) = serde_json::from_str::<Value>(&row.lines_json) {
            map.insert(row.report_period, lines);
        }
    }
    map
}

fn number(lines: &Map<String, Value>, key: &str) -> Option<f64> {
    lines.get(key).and_then(|value| value.as_f64())
}

fn period_parts(period: &str) -> Option<(i32, u32, u32)> {
    let date = NaiveDate::parse_from_str(period, "%Y-%m-%d").ok()?;
    Some((date.format("%Y").to_string().parse().ok()?, date.format("%m").to_string().parse().ok()?, date.format("%d").to_string().parse().ok()?))
}

/// 同年上一期（用于把累计数拆成单季）：03-31 → 空；06-30 → 03-31；09-30 → 06-30；12-31 → 09-30。
fn previous_period_in_year(period: &str) -> Option<String> {
    let (year, month, _) = period_parts(period)?;
    match month {
        3 => None,
        6 => Some(format!("{year}-03-31")),
        9 => Some(format!("{year}-06-30")),
        12 => Some(format!("{year}-09-30")),
        _ => None,
    }
}

fn same_quarter_last_year(period: &str) -> Option<String> {
    let (year, month, day) = period_parts(period)?;
    Some(format!("{:04}-{:02}-{:02}", year - 1, month, day))
}

fn single(series: &BTreeMap<String, Map<String, Value>>, period: &str, key: &str) -> Option<f64> {
    let cumulative = number(series.get(period)?, key)?;
    match previous_period_in_year(period) {
        Some(previous) => {
            let prior = number(series.get(&previous)?, key)?;
            Some(cumulative - prior)
        }
        None => Some(cumulative),
    }
}

/// 单季毛利率：营收与营业成本都取单季。
fn gross_margin(series: &BTreeMap<String, Map<String, Value>>, period: &str) -> Option<f64> {
    let revenue = single(series, period, "bizinco").or_else(|| single(series, period, "biztotinco"))?;
    if revenue.abs() < 1.0 {
        return None;
    }
    let cost = single(series, period, "bizcost")?;
    Some((revenue - cost) / revenue * 100.0)
}

fn prior_quarter(period: &str) -> Option<String> {
    let (year, month, _) = period_parts(period)?;
    match month {
        3 => Some(format!("{:04}-12-31", year - 1)),
        6 => Some(format!("{year}-03-31")),
        9 => Some(format!("{year}-06-30")),
        12 => Some(format!("{year}-09-30")),
        _ => None,
    }
}

pub fn compute(conn: &mut diesel::SqliteConnection, code: &str, period: &str) -> Option<Metrics> {
    let income = load_periods(conn, code, "Income");
    let cash = load_periods(conn, code, "CashFlow");
    if income.is_empty() {
        return None;
    }
    let mut metrics = Metrics {
        code: code.to_string(),
        report_period: period.to_string(),
        ..Default::default()
    };

    #[derive(diesel::QueryableByName)]
    struct AnnouncedRow {
        #[diesel(sql_type = Nullable<Text>)]
        announced_on: Option<String>,
    }
    metrics.announced_on = diesel::sql_query(
        "SELECT announced_on FROM financial_statement WHERE code = ?1 AND kind = 'Income' AND report_period = ?2",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(period)
    .get_result::<AnnouncedRow>(conn)
    .ok()
    .and_then(|row| row.announced_on);

    metrics.rev_single = single(&income, period, "bizinco").or_else(|| single(&income, period, "biztotinco"));
    metrics.np_single = single(&income, period, "parenetp");
    let net_single = single(&income, period, "netprofit");

    if let (Some(current), Some(previous)) = (metrics.rev_single, same_quarter_last_year(period)) {
        if let Some(prior) = single(&income, &previous, "bizinco").or_else(|| single(&income, &previous, "biztotinco")) {
            if prior.abs() > 1.0 {
                metrics.rev_yoy = Some((current - prior) / prior.abs() * 100.0);
            }
        }
    }
    if let (Some(current), Some(previous)) = (metrics.np_single, same_quarter_last_year(period)) {
        if let Some(prior) = single(&income, &previous, "parenetp") {
            if prior.abs() > 1.0 {
                metrics.np_yoy = Some((current - prior) / prior.abs() * 100.0);
            }
        }
    }
    metrics.gm_single = gross_margin(&income, period);
    if let Some(previous) = prior_quarter(period) {
        if let (Some(current), Some(prior)) = (metrics.gm_single, gross_margin(&income, &previous)) {
            metrics.gm_qoq = Some(current - prior);
        }
    }
    if !cash.is_empty() {
        metrics.ocf_single = single(&cash, period, "mananetr");
        if let (Some(ocf), Some(net)) = (metrics.ocf_single, net_single) {
            if net.abs() > 1.0 {
                metrics.ocf_ratio = Some(ocf / net);
            }
        }
    }
    if let (Some(np), Some(chg), Some(other), Some(dispose), Some(nonop_in), Some(nonop_out)) = (
        metrics.np_single,
        single(&income, period, "valuechgloss"),
        single(&income, period, "otherinco"),
        single(&income, period, "assetsdislinco"),
        single(&income, period, "nonoreve"),
        single(&income, period, "nonoexpe"),
    ) {
        if np.abs() > 1.0 {
            metrics.nonrec_ratio = Some((chg + other + dispose + nonop_in - nonop_out) / np);
        }
    }
    // 资产负债表当前源不可用（invalid evidence），不计算应收增速。
    metrics.balance_unavailable = true;
    Some(metrics)
}

pub fn advise(metrics: &Metrics) -> Advice {
    let rev_yoy = metrics.rev_yoy;
    let np_yoy = metrics.np_yoy;
    let ocf = metrics.ocf_single.unwrap_or(0.0);
    let ocf_ratio = metrics.ocf_ratio;
    let nonrec = metrics.nonrec_ratio;

    if let (Some(rev), Some(np)) = (rev_yoy, np_yoy) {
        if rev < -30.0 && np < -50.0 {
            return Advice {
                action: "清仓",
                ratio_pct: 100,
                reason: format!("单季营收同比 {rev:+.0}%、归母净利同比 {np:+.0}%，双杀且幅度大"),
            };
        }
    }
    if let Some(np) = np_yoy {
        if np < -30.0 {
            return Advice {
                action: "减半",
                ratio_pct: 50,
                reason: format!("单季归母净利同比 {np:+.0}%（< −30%）"),
            };
        }
    }
    if ocf < 0.0 {
        if let (Some(ratio), Some(np)) = (ocf_ratio, np_yoy) {
            if ratio < 0.0 && np < 0.0 {
                return Advice {
                    action: "减半",
                    ratio_pct: 50,
                    reason: format!("单季经营现金流为负、净利同比 {np:+.0}%（< 0），现金流与利润背离"),
                };
            }
        }
    }
    if let (Some(ratio), Some(nonrec_ratio)) = (ocf_ratio, nonrec) {
        if ratio < 0.0 && nonrec_ratio > 0.5 {
            return Advice {
                action: "减三成",
                ratio_pct: 30,
                reason: format!(
                    "经营现金流/净利 {ratio:.2}（<0）、非经常项占归母净利 {:.0}%（>50%）",
                    nonrec_ratio * 100.0
                ),
            };
        }
    }
    if let (Some(rev), Some(np), Some(ratio)) = (rev_yoy, np_yoy, ocf_ratio) {
        if rev > 20.0 && np > 30.0 && ratio > 0.8 {
            return Advice {
                action: "加仓",
                ratio_pct: 20,
                reason: format!("单季营收同比 {rev:+.0}%、归母净利同比 {np:+.0}%、经营现金流/净利 {ratio:.2}"),
            };
        }
    }
    Advice {
        action: "持有",
        ratio_pct: 0,
        reason: "指标未触发任何调整阈值".to_string(),
    }
}

fn fmt(value: Option<f64>, suffix: &str) -> String {
    match value {
        Some(number) => format!("{number:+.1}{suffix}"),
        None => "源缺失".to_string(),
    }
}

/// 落一条报告记录（同一票同一报告期幂等），用于审计与次日盘前复用。
pub fn record_report(
    conn: &mut diesel::SqliteConnection,
    metrics: &Metrics,
    advice: &Advice,
    rendered: &str,
    pushed: bool,
) -> Result<(), String> {
    diesel::sql_query(
        "INSERT INTO holding_earnings_report          (code, report_period, announced_on, advice, ratio_pct, reason, rendered, pushed_at)          VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CASE WHEN ?8 THEN CURRENT_TIMESTAMP ELSE NULL END)          ON CONFLICT(code, report_period) DO UPDATE SET            announced_on=excluded.announced_on, advice=excluded.advice, ratio_pct=excluded.ratio_pct,            reason=excluded.reason, rendered=COALESCE(NULLIF(excluded.rendered, ''), holding_earnings_report.rendered),            pushed_at=COALESCE(holding_earnings_report.pushed_at, excluded.pushed_at)",
    )
    .bind::<Text, _>(metrics.code.clone())
    .bind::<Text, _>(metrics.report_period.clone())
    .bind::<Nullable<Text>, _>(metrics.announced_on.clone())
    .bind::<Text, _>(advice.action)
    .bind::<Integer, _>(advice.ratio_pct)
    .bind::<Text, _>(advice.reason.clone())
    .bind::<Text, _>(rendered.to_string())
    .bind::<Integer, _>(i32::from(pushed))
    .execute(conn)
    .map_err(|error| format!("财报报告落库失败: {error}"))?;
    Ok(())
}

pub fn render(metrics: &Metrics, advice: &Advice, virtual_position: bool) -> String {
    let tag = if virtual_position { "〔虚拟〕" } else { "〔持仓〕" };
    let period = match period_parts(&metrics.report_period) {
        Some((year, month, _)) => match month {
            3 => format!("{year}Q1"),
            6 => format!("{year}H1"),
            9 => format!("{year}Q3"),
            _ => format!("{year}年报"),
        },
        None => metrics.report_period.clone(),
    };
    let mut out = format!("{tag}{} {period}", metrics.code);
    out.push_str(&format!(
        "\n  单季营收 {} ｜ 归母净利 {} ｜ 毛利率 {}（环比 {}）",
        fmt(metrics.rev_yoy, "%"),
        fmt(metrics.np_yoy, "%"),
        fmt(metrics.gm_single, "%"),
        fmt(metrics.gm_qoq, "pct")
    ));
    out.push_str(&format!(
        "\n  经营现金流/净利 {} ｜ 非经常项占归母 {}",
        fmt(metrics.ocf_ratio, ""),
        match metrics.nonrec_ratio {
            Some(value) => format!("{:+.0}%", value * 100.0),
            None => "源缺失".to_string(),
        }
    ));
    if metrics.balance_unavailable {
        out.push_str("\n  应收增速：资产负债表源不可用（未计算）");
    }
    out.push_str(&format!(
        "\n  → 建议：{} {}%（{}）",
        advice.action, advice.ratio_pct, advice.reason
    ));
    if let Some(announced) = &metrics.announced_on {
        out.push_str(&format!("\n  披露：{announced}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(pairs: &[(&str, f64)]) -> Map<String, Value> {
        let mut map = Map::new();
        for (key, value) in pairs {
            map.insert(key.to_string(), json!(value));
        }
        map
    }

    #[test]
    fn single_quarter_is_cumulative_difference() {
        let mut series = BTreeMap::new();
        series.insert("2026-06-30".to_string(), lines(&[("bizinco", 600.0)]));
        series.insert("2026-03-31".to_string(), lines(&[("bizinco", 100.0)]));
        assert_eq!(single(&series, "2026-06-30", "bizinco"), Some(500.0));
    }

    #[test]
    fn advice_hits_each_threshold() {
        let mut metrics = Metrics { rev_yoy: Some(-40.0), np_yoy: Some(-60.0), ..Default::default() };
        assert_eq!(advise(&metrics).action, "清仓");
        metrics = Metrics { np_yoy: Some(-35.0), ..Default::default() };
        assert_eq!(advise(&metrics).action, "减半");
        metrics = Metrics { np_yoy: Some(-5.0), ocf_single: Some(-100.0), ocf_ratio: Some(-1.0), ..Default::default() };
        assert_eq!(advise(&metrics).action, "减半");
        metrics = Metrics { ocf_single: Some(-10.0), ocf_ratio: Some(-0.5), nonrec_ratio: Some(0.8), ..Default::default() };
        assert_eq!(advise(&metrics).action, "减三成");
        metrics = Metrics { rev_yoy: Some(25.0), np_yoy: Some(40.0), ocf_ratio: Some(0.9), ..Default::default() };
        assert_eq!(advise(&metrics).action, "加仓");
        metrics = Metrics { rev_yoy: Some(5.0), np_yoy: Some(5.0), ocf_ratio: Some(1.2), ..Default::default() };
        assert_eq!(advise(&metrics).action, "持有");
    }
}
