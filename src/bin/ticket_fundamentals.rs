//! 6.6 财务事实预取（只读 ExternalV1，写本地缓存）。
//!
//! 用法：
//!   cargo run --bin ticket_fundamentals -- --codes 001216,002131 --date 2026-11-10
//!
//! 输出：`data/fundamentals/<date>/<code>.json`，每个 kind 只保留最新报告期，
//! `lines` 原样保留（缺失值是 null，不补 0）；`fiscal_period` 不伪造。

use std::collections::BTreeMap;
use std::path::PathBuf;

use stock_analysis::market_domain::StatementKind;

fn exchange_for(code: &str) -> &'static str {
    match code.chars().next() {
        Some('6') => "Shanghai",
        Some('4') | Some('8') => "Beijing",
        _ => "Shenzhen",
    }
}

fn instrument(code: &str) -> serde_json::Value {
    serde_json::json!({
        "exchange": exchange_for(code),
        "code": code,
        "asset_class": "Equity",
    })
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let mut codes: Vec<String> = Vec::new();
    let mut date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--codes" => {
                let value = args.next().ok_or("--codes 需要逗号分隔的代码")?;
                codes = value
                    .split(',')
                    .map(str::trim)
                    .filter(|code| !code.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            "--date" => date = args.next().ok_or("--date 需要 YYYY-MM-DD")?,
            other => return Err(format!("未知参数: {other}")),
        }
    }
    if codes.is_empty() {
        return Err("用法: ticket_fundamentals --codes 001216,002131 [--date YYYY-MM-DD]".into());
    }
    if codes.len() > 10 {
        return Err("单次最多 10 只（ExternalV1 合同上限）".into());
    }

fn line_value(lines: &[serde_json::Value], key: &str) -> Option<f64> {
    lines
        .iter()
        .find(|line| line.get("key").and_then(|value| value.as_str()) == Some(key))
        .and_then(|line| line.get("value"))
        .and_then(|value| value.as_f64())
}

fn lines_of(record: &serde_json::Value) -> &[serde_json::Value] {
    record
        .get("lines")
        .and_then(|lines| lines.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn ratio(numerator: Option<f64>, denominator: Option<f64>) -> Option<f64> {
    match (numerator, denominator) {
        (Some(numerator), Some(denominator)) if denominator.abs() > f64::EPSILON => {
            Some(numerator / denominator)
        }
        _ => None,
    }
}

fn growth(current: Option<f64>, previous: Option<f64>) -> Option<f64> {
    match (current, previous) {
        (Some(current), Some(previous)) if previous.abs() > f64::EPSILON => {
            Some(current / previous - 1.0)
        }
        _ => None,
    }
}

fn metric(
    name: &str,
    value: Option<f64>,
    reason: &str,
    reasons: &mut BTreeMap<String, String>,
) -> serde_json::Value {
    if value.is_none() {
        reasons.insert(name.to_owned(), reason.to_owned());
    }
    serde_json::json!(value)
}

/// 6.6：只计算输入完整且定义明确的指标；任一输入缺失就 null + reason，不插值。
fn prior_year_record<'a>(
    records: &'a [serde_json::Value],
    period: &str,
) -> Option<&'a serde_json::Value> {
    use chrono::Datelike;
    let date = chrono::NaiveDate::parse_from_str(period, "%Y-%m-%d").ok()?;
    let target = chrono::NaiveDate::from_ymd_opt(date.year() - 1, date.month(), date.day())?;
    let target_text = target.to_string();
    records
        .iter()
        .find(|record| record["report_period"].as_str() == Some(target_text.as_str()))
}

fn combined_metrics(
    kinds: &BTreeMap<String, serde_json::Value>,
) -> (serde_json::Value, BTreeMap<String, String>) {
    let income = kinds.get("Income").map(lines_of).unwrap_or(&[]);
    let balance = kinds.get("Balance").map(lines_of).unwrap_or(&[]);
    let cash = kinds.get("CashFlow").map(lines_of).unwrap_or(&[]);
    let mut reasons = BTreeMap::new();
    let mut metrics = serde_json::Map::new();
    for (label, key) in [
        ("Income", "revenue_yoy"),
        ("Income", "net_profit_yoy"),
        ("Income", "gross_margin"),
        ("Balance", "debt_ratio"),
    ] {
        let value = kinds
            .get(label)
            .and_then(|kind| kind.get("metrics"))
            .and_then(|metrics| metrics.get(key))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        if value.is_null() {
            if let Some(reason) = kinds
                .get(label)
                .and_then(|kind| kind.get("metric_reasons"))
                .and_then(|reasons| reasons.get(key))
                .and_then(|value| value.as_str())
            {
                reasons.insert(key.to_owned(), reason.to_owned());
            }
        }
        metrics.insert(key.to_owned(), value);
    }
    let net_profit = line_value(income, "netprofit");
    let revenue = line_value(income, "bizinco");
    let cost = line_value(income, "bizcost");
    let inventory = line_value(balance, "inve");
    let ocf = line_value(cash, "mananetr");
    let ocf_to_net_profit = if net_profit.is_some_and(|value| value > 0.0) {
        ratio(ocf, net_profit)
    } else {
        None
    };
    let receivables_to_revenue = if revenue.is_some_and(|value| value > 0.0) {
        ratio(line_value(balance, "accorece"), revenue)
    } else {
        None
    };
    let inventory_turnover = if inventory.is_some_and(|value| value > 0.0) {
        ratio(cost, inventory)
    } else {
        None
    };
    metrics.insert(
        "ocf_to_net_profit".to_owned(),
        metric(
            "ocf_to_net_profit",
            ocf_to_net_profit,
            "missing_ocf_or_net_profit_non_positive",
            &mut reasons,
        ),
    );
    metrics.insert(
        "receivables_to_revenue".to_owned(),
        metric(
            "receivables_to_revenue",
            receivables_to_revenue,
            "missing_receivables_or_revenue_non_positive",
            &mut reasons,
        ),
    );
    metrics.insert(
        "inventory_turnover".to_owned(),
        metric(
            "inventory_turnover",
            inventory_turnover,
            "missing_cost_or_inventory_non_positive",
            &mut reasons,
        ),
    );
    (serde_json::Value::Object(metrics), reasons)
}

fn build_kind_entry(
    mut latest: serde_json::Value,
    records: &[serde_json::Value],
) -> serde_json::Value {
    let period = latest["report_period"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let prior = prior_year_record(records, &period);
    let prior_year_period = prior
        .and_then(|record| record["report_period"].as_str())
        .map(str::to_owned);
    let (metrics, metric_reasons, missing_lines) = compute_metrics(&latest, prior);
    latest["metrics"] = metrics;
    latest["metric_reasons"] = serde_json::json!(metric_reasons);
    latest["missing_lines"] = serde_json::json!(missing_lines);
    latest["prior_year_period"] = serde_json::json!(prior_year_period);
    latest
}

/// 6.6：只计算输入完整且定义明确的指标；任一输入缺失就 null + reason，不插值。
fn compute_metrics(
    latest: &serde_json::Value,
    prior: Option<&serde_json::Value>,
) -> (
    serde_json::Value,
    BTreeMap<String, String>,
    Vec<String>,
) {
    use chrono::Datelike;

    let lines = lines_of(latest);
    let prior_lines = prior.map(lines_of).unwrap_or(&[]);
    let mut reasons = BTreeMap::new();
    let revenue = line_value(lines, "bizinco");
    let cost = line_value(lines, "bizcost");
    let net_profit = line_value(lines, "netprofit");
    let ocf = line_value(lines, "mananetr");
    let receivables = line_value(lines, "accorece");
    let inventory = line_value(lines, "inve");
    let total_assets = line_value(lines, "totasset");
    let total_liabilities = line_value(lines, "totliab");

    let revenue_yoy = growth(revenue, line_value(prior_lines, "bizinco"));
    let net_profit_yoy = growth(net_profit, line_value(prior_lines, "netprofit"));
    let ocf_to_net_profit = if net_profit.is_some_and(|value| value > 0.0) {
        ratio(ocf, net_profit)
    } else {
        None
    };
    let receivables_to_revenue = if revenue.is_some_and(|value| value > 0.0) {
        ratio(receivables, revenue)
    } else {
        None
    };
    let inventory_turnover = if inventory.is_some_and(|value| value > 0.0) {
        ratio(cost, inventory)
    } else {
        None
    };
    let debt_ratio = if total_assets.is_some_and(|value| value > 0.0) {
        ratio(total_liabilities, total_assets)
    } else {
        None
    };
    let gross_margin = match (revenue, cost) {
        (Some(revenue), Some(cost)) if revenue > 0.0 => Some((revenue - cost) / revenue),
        _ => None,
    };

    let metrics = serde_json::json!({
        "revenue_yoy": metric("revenue_yoy", revenue_yoy, "missing_prior_year_or_revenue", &mut reasons),
        "net_profit_yoy": metric("net_profit_yoy", net_profit_yoy, "missing_prior_year_or_net_profit", &mut reasons),
        "ocf_to_net_profit": metric("ocf_to_net_profit", ocf_to_net_profit, "missing_ocf_or_net_profit_non_positive", &mut reasons),
        "receivables_to_revenue": metric("receivables_to_revenue", receivables_to_revenue, "missing_receivables_or_revenue_non_positive", &mut reasons),
        "inventory_turnover": metric("inventory_turnover", inventory_turnover, "missing_cost_or_inventory_non_positive", &mut reasons),
        "debt_ratio": metric("debt_ratio", debt_ratio, "missing_liabilities_or_total_assets_non_positive", &mut reasons),
        "gross_margin": metric("gross_margin", gross_margin, "missing_revenue_or_cost", &mut reasons),
    });

    let missing_lines: Vec<String> = lines
        .iter()
        .filter(|line| line.get("value").map(|value| value.is_null()).unwrap_or(true))
        .filter_map(|line| line.get("key").and_then(|value| value.as_str()).map(str::to_owned))
        .collect();

    (metrics, reasons, missing_lines)
}

    let instruments: Vec<_> = codes.iter().map(|code| instrument(code)).collect();
    let mut per_code: BTreeMap<String, BTreeMap<String, serde_json::Value>> =
        codes.iter().map(|code| (code.clone(), BTreeMap::new())).collect();
    let mut batch_ids: BTreeMap<String, String> = BTreeMap::new();

    for kind in [StatementKind::Balance, StatementKind::Income, StatementKind::CashFlow] {
        let batch = stock_analysis::data_gateway::grpc_source::GrpcSource::financial_statements_external_prefetch(
            instruments.clone(),
            kind,
        )
        .await
        .map_err(|error| format!("{kind:?} 拉取失败: {error}"))?;
        let label = match kind {
            StatementKind::Balance => "Balance",
            StatementKind::Income => "Income",
            StatementKind::CashFlow => "CashFlow",
        };
        let mut by_code: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
        for record in batch.records() {
            let code = record.instrument.code().to_owned();
            let period = record.report_period.to_string();
            let candidate = serde_json::json!({
                "status": "ok",
                "report_period": period,
                "announced_on": record.announced_on.clone().map(|date| date.to_string()),
                "currency": record.currency.as_ref().map(|value| value.as_str()),
                "lines": record.lines,
                "evidence": record.evidence,
            });
            by_code.entry(code).or_default().push(candidate);
        }
        for (code, mut records) in by_code {
            records.sort_by(|left, right| {
                right["report_period"]
                    .as_str()
                    .cmp(&left["report_period"].as_str())
            });
            let latest = records[0].clone();
            let value = build_kind_entry(latest, &records);
            if let Some(slot) = per_code.get_mut(&code) {
                slot.insert(label.to_owned(), value);
            }
        }
        batch_ids.insert(label.to_owned(), batch.evidence().batch_id.clone());
    }

    let captured_at = chrono::Local::now().to_rfc3339();
    let dir = PathBuf::from("data").join("fundamentals").join(&date);
    std::fs::create_dir_all(&dir).map_err(|error| format!("创建 {} 失败: {error}", dir.display()))?;
    for (code, kinds) in &per_code {
        let mut missing = Vec::new();
        for label in ["Balance", "Income", "CashFlow"] {
            if !kinds.contains_key(label) {
                missing.push(format!("{label}:report_not_available"));
            }
        }
        let (metrics, metric_reasons) = combined_metrics(kinds);
        let payload = serde_json::json!({
            "schema": "ticket-fundamentals-v1",
            "business_date": date,
            "code": code,
            "captured_at": captured_at,
            "kinds": kinds,
            "metrics": metrics,
            "metric_reasons": metric_reasons,
            "metric_notes": {
                "revenue_yoy": "latest report period vs same period prior year",
                "net_profit_yoy": "latest report period vs same period prior year",
                "ocf_to_net_profit": "period operating cash flow / net profit; null when net profit <= 0",
                "receivables_to_revenue": "period-end accounts receivable / period revenue",
                "inventory_turnover": "period operating cost / period-end inventory; not annualized",
                "debt_ratio": "period-end total liabilities / total assets",
                "gross_margin": "(revenue - operating cost) / revenue",
            },
            "batch_ids": batch_ids,
            "missing": missing,
        });
        let path = dir.join(format!("{code}.json"));
        std::fs::write(&path, serde_json::to_vec_pretty(&payload).map_err(|e| e.to_string())?)
            .map_err(|error| format!("写入 {} 失败: {error}", path.display()))?;
        println!(
            "{} kinds={} missing={} -> {}",
            code,
            kinds.len(),
            missing.len(),
            path.display()
        );
    }
    Ok(())
}
