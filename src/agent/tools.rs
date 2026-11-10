//! 财务/估值取数工具。
//!
//! 2026-11-10 改造（深度研判数据缺口）：
//! - 财务：上游 `FinancialStatements` 只给原始报表行，不含 ROE/毛利率/同比比率
//!   （Windows 回执 GD-DR-02）。这里按**明确公式**在客户端派生，并把公式写进返回值，
//!   同时保留 provider/batch_id，便于回链；不再出现"财务工具拿不到比率"的局面。
//! - 估值：`MarketStatistics` 提供 PE(TTM)/PB/总市值/流通市值/涨跌停价/量比，
//!   一并注入，补上日线视图只有 10 个字段的缺口。
//! - 每一块缺失都显式标注（MISSING+原因），两块都缺才整体失败。

use crate::agent::tool::Tool;
use crate::data_gateway::{CompanyDataGateway, GatewayBatch};
use crate::market_domain::{FinancialStatement, MarketStatistics};
use async_trait::async_trait;
use serde_json::json;

/// 上游原始行 key 的多来源别名（Sina 风格 / Hithink 风格）。
const REVENUE_KEYS: [&str; 3] = ["bizinco", "operating_income", "revenue"];
const COST_KEYS: [&str; 3] = ["bizcost", "operating_costs", "operating_cost"];
const NET_PROFIT_KEYS: [&str; 4] = [
    "parenetp",
    "parent_holder_net_profit",
    "netprofit",
    "net_profit",
];
const EPS_KEYS: [&str; 3] = ["basiceps", "basic_eps", "eps"];

const RATIO_FORMULA: &str =
    "毛利率=(营业收入-营业成本)/营业收入; 净利率=归母净利/营业收入; 同比=同报告期上年同期相对变化";

pub struct FetchFinancialTool;

impl Default for FetchFinancialTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FetchFinancialTool {
    pub fn new() -> Self {
        Self
    }
}

fn line_value(statement: &FinancialStatement, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        statement
            .lines
            .iter()
            .find(|line| line.key.as_str() == *key)
            .and_then(|line| line.value.map(|value| value.get()))
    })
}

#[derive(Debug, Clone, PartialEq)]
struct PeriodFacts {
    report_period: String,
    eps: Option<f64>,
    revenue: Option<f64>,
    net_profit: Option<f64>,
    gross_margin: Option<f64>,
    net_margin: Option<f64>,
    revenue_yoy: Option<f64>,
    net_profit_yoy: Option<f64>,
}

fn period_facts(statement: &FinancialStatement) -> PeriodFacts {
    let revenue = line_value(statement, &REVENUE_KEYS);
    let cost = line_value(statement, &COST_KEYS);
    let net_profit = line_value(statement, &NET_PROFIT_KEYS);
    let gross_margin = match (revenue, cost) {
        (Some(revenue), Some(cost)) if revenue.abs() > 1e-9 => {
            Some((revenue - cost) / revenue * 100.0)
        }
        _ => None,
    };
    let net_margin = match (revenue, net_profit) {
        (Some(revenue), Some(net)) if revenue.abs() > 1e-9 => Some(net / revenue * 100.0),
        _ => None,
    };
    PeriodFacts {
        report_period: statement.report_period.as_str().to_string(),
        eps: line_value(statement, &EPS_KEYS),
        revenue,
        net_profit,
        gross_margin,
        net_margin,
        revenue_yoy: None,
        net_profit_yoy: None,
    }
}

fn yoy_pct(current: Option<f64>, prior: Option<f64>) -> Option<f64> {
    match (current, prior) {
        (Some(current), Some(prior)) if prior.abs() > 1e-9 => {
            Some((current - prior) / prior.abs() * 100.0)
        }
        _ => None,
    }
}

/// 同报告期上年同期的 key（2026-09-30 → 2025-09-30）。
fn prior_year_period(period: &str) -> Option<String> {
    let (year, rest) = period.split_once('-')?;
    let year: i32 = year.parse().ok()?;
    Some(format!("{}-{}", year - 1, rest))
}

/// 按报告期降序，逐期派生毛利率/净利率/同比。同比只与同报告期的上年同期比较。
fn derive_periods(records: &[FinancialStatement]) -> Vec<PeriodFacts> {
    let mut facts: Vec<PeriodFacts> = records.iter().map(period_facts).collect();
    facts.sort_by(|left, right| right.report_period.cmp(&left.report_period));
    facts.dedup_by(|left, right| left.report_period == right.report_period);
    let by_period: std::collections::HashMap<String, PeriodFacts> = facts
        .iter()
        .map(|fact| (fact.report_period.clone(), fact.clone()))
        .collect();
    for fact in facts.iter_mut() {
        if let Some(prior) = prior_year_period(&fact.report_period)
            .and_then(|period| by_period.get(&period).cloned())
        {
            fact.revenue_yoy = yoy_pct(fact.revenue, prior.revenue);
            fact.net_profit_yoy = yoy_pct(fact.net_profit, prior.net_profit);
        }
    }
    facts
}

fn period_json(fact: &PeriodFacts) -> serde_json::Value {
    json!({
        "report_period": fact.report_period,
        "eps": fact.eps,
        "revenue_yi": fact.revenue.map(|value| round2(value / 1e8)),
        "net_profit_yi": fact.net_profit.map(|value| round2(value / 1e8)),
        "gross_margin_pct": fact.gross_margin.map(round2),
        "net_margin_pct": fact.net_margin.map(round2),
        "revenue_yoy_pct": fact.revenue_yoy.map(round2),
        "net_profit_yoy_pct": fact.net_profit_yoy.map(round2),
    })
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn miyi(value: f64) -> f64 {
    round2(value / 1e8)
}

fn income_block(result: Result<GatewayBatch<FinancialStatement>, String>) -> serde_json::Value {
    match result {
        Ok(GatewayBatch::Available { records, evidence }) if !records.is_empty() => {
            let periods = derive_periods(&records);
            let latest = periods.first().map(period_json);
            json!({
                "status": "OK",
                "provider": format!("{:?}", evidence.provider),
                "batch_id": evidence.batch_id,
                "source_at": evidence.source_at,
                "formula": RATIO_FORMULA,
                "latest": latest,
                "periods_desc": periods.iter().take(8).map(period_json).collect::<Vec<_>>(),
            })
        }
        Ok(GatewayBatch::Available { evidence, .. })
        | Ok(GatewayBatch::VerifiedEmpty(evidence)) => json!({
            "status": "MISSING",
            "reason": "上游返回空财报批次",
            "batch_id": evidence.batch_id,
        }),
        Err(reason) => json!({ "status": "MISSING", "reason": reason }),
    }
}

fn valuation_block(result: Result<GatewayBatch<MarketStatistics>, String>) -> serde_json::Value {
    match result {
        Ok(GatewayBatch::Available { records, evidence }) => match records.first() {
            Some(stats) => json!({
                "status": "OK",
                "provider": format!("{:?}", evidence.provider),
                "batch_id": evidence.batch_id,
                "pe_ttm": stats.trailing_pe().map(|value| round2(value.get())),
                "pe_static": stats.static_pe().map(|value| round2(value.get())),
                "pb": stats.pb().map(|value| round2(value.get())),
                "total_market_cap_yi": stats.total_market_cap().map(|value| miyi(value.get())),
                "float_market_cap_yi": stats.floating_market_cap().map(|value| miyi(value.get())),
                "upper_limit": stats.upper_limit().map(|value| round2(value.get())),
                "lower_limit": stats.lower_limit().map(|value| round2(value.get())),
                "volume_ratio": stats.volume_ratio().map(|value| round2(value.get())),
            }),
            None => json!({ "status": "MISSING", "reason": "估值批次无记录" }),
        },
        Ok(GatewayBatch::VerifiedEmpty(evidence)) => json!({
            "status": "MISSING",
            "reason": "上游返回空估值批次",
            "batch_id": evidence.batch_id,
        }),
        Err(reason) => json!({ "status": "MISSING", "reason": reason }),
    }
}

fn batch_evidence_note(income: &serde_json::Value, valuation: &serde_json::Value) -> serde_json::Value {
    json!({
        "income_batch_id": income.get("batch_id").cloned().unwrap_or(serde_json::Value::Null),
        "valuation_batch_id": valuation.get("batch_id").cloned().unwrap_or(serde_json::Value::Null),
    })
}

#[async_trait]
impl Tool for FetchFinancialTool {
    fn name(&self) -> &str {
        "fetch_financials"
    }

    fn description(&self) -> &str {
        "获取财报原始行与市场估值：财务比率（毛利率/净利率/营收净利同比）由客户端按固定公式派生并标注公式；估值含 PE(TTM)/PB/总市值/流通市值/涨跌停价/量比。缺失项显式标注 MISSING。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "股票代码，如 '600519' 或 '000001'"
                }
            },
            "required": ["code"]
        })
    }

    async fn call(&self, input: serde_json::Value) -> anyhow::Result<String> {
        let code = input
            .get("code")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' parameter"))?;
        let gateway = CompanyDataGateway::new();
        let codes = vec![code.to_string()];
        // 桥端 BR-159 审计写库偶发 database is locked / observation_stale（实测同一次
        // 请求第 1、2 次失败、第 3 次成功）。取数做 3 次有界重试，不做无限退避。
        let mut income_result = gateway.income_statements(&codes).await;
        for _ in 0..2 {
            if income_result.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            income_result = gateway.income_statements(&codes).await;
        }
        let mut valuation_result = gateway.market_statistics(&codes).await;
        for _ in 0..2 {
            if valuation_result.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            valuation_result = gateway.market_statistics(&codes).await;
        }
        let income = income_block(income_result.map_err(|error| format!("{error:#}")));
        let valuation = valuation_block(valuation_result.map_err(|error| format!("{error:#}")));
        if income["status"] != "OK" && valuation["status"] != "OK" {
            let reason = income["reason"].as_str().unwrap_or("财报与估值均不可用");
            anyhow::bail!("No financial or valuation records found for {code}: {reason}");
        }
        let evidence = batch_evidence_note(&income, &valuation);
        Ok(json!({
            "code": code,
            "income": income,
            "valuation": valuation,
            "evidence": evidence,
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::BatchEvidence;
    use crate::market_domain::{
        AssetClass, Exchange, FiniteNumber, InstrumentId, IsoDate, NonEmptyText, ProviderId,
        SourceEvidence, StatementKind,
    };

    fn statement(period: &str, revenue: f64, cost: f64, net_profit: f64, eps: f64) -> FinancialStatement {
        let line = |key: &str, label: &str, value: f64| crate::market_domain::FinancialLine {
            key: NonEmptyText::new(key).expect("key"),
            source_label: NonEmptyText::new(label).expect("label"),
            value: Some(FiniteNumber::new(value).expect("finite")),
            unit: None,
        };
        FinancialStatement {
            instrument: InstrumentId::new(
                Exchange::Shanghai,
                "TEST_CODE_600519",
                AssetClass::Equity,
            )
            .expect("instrument"),
            kind: StatementKind::Income,
            report_period: IsoDate::new(period).expect("period"),
            announced_on: None,
            currency: Some(NonEmptyText::new("CNY").expect("currency")),
            lines: vec![
                line("bizinco", "营业收入", revenue),
                line("bizcost", "营业成本", cost),
                line("parenetp", "归属于母公司股东的净利润", net_profit),
                line("basiceps", "基本每股收益", eps),
            ],
            evidence: SourceEvidence::new(ProviderId::Sina, "1785799979.851045000", "TEST_BATCH")
                .expect("evidence"),
        }
    }

    #[test]
    fn derived_ratios_and_same_period_yoy_are_exact() {
        let records = vec![
            statement("2026-09-30", 1000.0, 600.0, 100.0, 0.5),
            statement("2025-09-30", 800.0, 560.0, 40.0, 0.2),
        ];
        let periods = derive_periods(&records);
        let latest = &periods[0];
        assert_eq!(latest.report_period, "2026-09-30");
        assert_eq!(latest.gross_margin, Some(40.0));
        assert_eq!(latest.net_margin, Some(10.0));
        assert_eq!(latest.revenue_yoy, Some(25.0));
        assert_eq!(latest.net_profit_yoy, Some(150.0));
        // 上年同期没有更早一期时，同比必须留空而不是编造
        assert_eq!(periods[1].revenue_yoy, None);
    }

    #[test]
    fn missing_components_stay_none_instead_of_zero() {
        let mut record = statement("2026-09-30", 1000.0, 600.0, 100.0, 0.5);
        record.lines.retain(|line| line.key.as_str() != "bizcost");
        let periods = derive_periods(&[record]);
        assert_eq!(periods[0].gross_margin, None);
        assert_eq!(periods[0].net_margin, Some(10.0));
    }

    #[test]
    fn blank_income_batch_is_reported_as_missing_not_ok() {
        let evidence = BatchEvidence {
            provider: ProviderId::Sina,
            source: "TEST_SOURCE".to_string(),
            source_at: None,
            observed_at: "1785799979.851045000".to_string(),
            batch_id: "TEST_BATCH".to_string(),
        };
        let block = income_block(Ok(GatewayBatch::VerifiedEmpty(evidence)));
        assert_eq!(block["status"], "MISSING");
    }
}
