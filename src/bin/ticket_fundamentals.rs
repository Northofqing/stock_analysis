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
        let mut latest: BTreeMap<String, serde_json::Value> = BTreeMap::new();
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
            latest
                .entry(code)
                .and_modify(|current| {
                    if candidate["report_period"].as_str() > current["report_period"].as_str() {
                        *current = candidate.clone();
                    }
                })
                .or_insert(candidate);
        }
        for (code, value) in latest {
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
        let payload = serde_json::json!({
            "schema": "ticket-fundamentals-v1",
            "business_date": date,
            "code": code,
            "captured_at": captured_at,
            "kinds": kinds,
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
