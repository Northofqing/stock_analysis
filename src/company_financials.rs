//! BR-115/BR-137/BR-168/BR-210 financial-metrics model and conservative projection.
//!
//! Acquisition belongs to [`crate::data_gateway::CompanyDataGateway`].  This
//! module only projects facts whose semantics are explicit in the admitted
//! normalized statement batch. Missing ratios and growth fields remain absent.

use anyhow::{anyhow, Result};
use magic_market_core::{ProviderId, StatementKind};
use sha2::{Digest, Sha256};

use crate::data_gateway::{company::FinancialStatement, parse_evidence_instant, GatewayBatch};

/// Immutable identity of the complete Gateway batch admitted by this
/// projection. Provider time remains optional and is never replaced with a
/// local timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinancialProjectionEvidence {
    pub provider: ProviderId,
    pub source: String,
    pub source_at: Option<String>,
    pub observed_at: String,
    pub batch_id: String,
    pub content_sha256: String,
}

#[derive(Debug, Clone, Default)]
pub struct FinancialPeriod {
    pub report_date: Option<String>,
    pub eps: Option<f64>,
    pub roe: Option<f64>,
    pub revenue_yoy: Option<f64>,
    pub net_profit_yoy: Option<f64>,
    pub gross_margin: Option<f64>,
    pub net_margin: Option<f64>,
    pub op_cash_flow_ps: Option<f64>,
    pub total_asset_turnover: Option<f64>,
    pub debt_to_assets: Option<f64>,
}

impl FinancialPeriod {
    pub fn any(&self) -> bool {
        self.eps.is_some()
            || self.roe.is_some()
            || self.revenue_yoy.is_some()
            || self.net_profit_yoy.is_some()
            || self.gross_margin.is_some()
            || self.net_margin.is_some()
            || self.op_cash_flow_ps.is_some()
            || self.total_asset_turnover.is_some()
            || self.debt_to_assets.is_some()
    }

    pub fn equity_multiplier(&self) -> Option<f64> {
        self.debt_to_assets.and_then(|debt| {
            let equity_ratio = 1.0 - debt / 100.0;
            (equity_ratio > 1e-6).then_some(1.0 / equity_ratio)
        })
    }

    pub fn dupont(&self) -> Option<(f64, f64, f64, f64)> {
        let margin = self.net_margin?;
        let turnover = self.total_asset_turnover?;
        let multiplier = self.equity_multiplier()?;
        Some((margin, turnover, multiplier, margin * turnover * multiplier))
    }

    pub fn cfo_to_ni_ratio(&self) -> Option<f64> {
        match (self.op_cash_flow_ps, self.eps) {
            (Some(cash_flow), Some(eps)) if eps.abs() > 1e-6 => Some(cash_flow / eps),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct QualityReport {
    pub risk_score: u32,
    pub flags: Vec<String>,
    pub level: &'static str,
}

pub fn assess_quality(history: &[FinancialPeriod]) -> Option<QualityReport> {
    let latest = history.first()?;
    let mut flags = Vec::new();
    let mut score = 0_u32;

    if let (Some(profit), Some(revenue), Some(ratio)) = (
        latest.net_profit_yoy,
        latest.revenue_yoy,
        latest.cfo_to_ni_ratio(),
    ) {
        if profit - revenue > 20.0 && ratio < 0.5 {
            flags.push(format!(
                "净利增速({profit:.1}%)远高于营收增速({revenue:.1}%)且 CFO/NI={ratio:.2} 偏低 → 应计利润可疑"
            ));
            score += 25;
        }
    }
    if let Some(previous) = history.get(1) {
        if let (Some(current), Some(previous)) = (latest.gross_margin, previous.gross_margin) {
            let difference = current - previous;
            if difference.abs() > 5.0 {
                flags.push(format!(
                    "毛利率单期突变 {difference:+.2}pp（{previous:.2}% → {current:.2}%）→ 成本/口径异常"
                ));
                score += 15;
            }
        }
        if let (Some(current), Some(previous)) =
            (latest.cfo_to_ni_ratio(), previous.cfo_to_ni_ratio())
        {
            if previous >= 0.8 && current < 0.3 {
                flags.push(format!(
                    "CFO/NI 单期骤降 {previous:.2} → {current:.2}（盈利含金量突恶化）"
                ));
                score += 20;
            }
        }
    }
    if latest.net_profit_yoy.is_some_and(|value| value > 150.0) {
        flags.push("净利 YoY 过高 → 警惕基数效应/非经常性损益".into());
        score += 10;
    }
    if latest.revenue_yoy.is_some_and(|value| value > 100.0) {
        flags.push("营收 YoY 过高 → 警惕一次性合并/口径调整".into());
        score += 10;
    }

    let ratios: Vec<f64> = history
        .iter()
        .take(4)
        .filter_map(FinancialPeriod::cfo_to_ni_ratio)
        .collect();
    if ratios.len() >= 3 {
        let average = ratios.iter().sum::<f64>() / ratios.len() as f64;
        if average < 0.3 {
            flags.push(format!(
                "近{}期 CFO/NI 均值仅 {average:.2} → 长期盈利质量低",
                ratios.len()
            ));
            score += 15;
        }
    }

    let score = score.min(100);
    let level = if score >= 60 {
        "高风险⚠️"
    } else if score >= 30 {
        "需关注"
    } else if score > 0 {
        "轻微提示"
    } else {
        "无明显异常"
    };
    Some(QualityReport {
        risk_score: score,
        flags,
        level,
    })
}

#[derive(Debug, Clone, Default)]
pub struct Financials {
    pub report_date: Option<String>,
    pub published_date: Option<String>,
    pub eps: Option<f64>,
    pub roe: Option<f64>,
    pub revenue_yoy: Option<f64>,
    pub net_profit_yoy: Option<f64>,
    pub gross_margin: Option<f64>,
    pub net_margin: Option<f64>,
    pub source: Option<String>,
    pub evidence: Option<FinancialProjectionEvidence>,
    pub history: Vec<FinancialPeriod>,
}

impl Financials {
    pub fn any(&self) -> bool {
        self.eps.is_some()
            || self.roe.is_some()
            || self.revenue_yoy.is_some()
            || self.net_profit_yoy.is_some()
            || self.gross_margin.is_some()
            || self.net_margin.is_some()
    }

    /// Reject projections that lost or contradicted their admitted Gateway
    /// evidence before they enter a process cache or source-event adapter.
    pub fn require_projection_evidence(&self) -> Result<&FinancialProjectionEvidence> {
        let evidence = self
            .evidence
            .as_ref()
            .ok_or_else(|| anyhow!("BR-158 financial projection evidence is absent"))?;
        if evidence.source.trim().is_empty()
            || evidence.observed_at.trim().is_empty()
            || evidence.batch_id.trim().is_empty()
            || evidence.content_sha256.len() != 64
            || !evidence
                .content_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(anyhow!(
                "BR-158 financial projection evidence is incomplete"
            ));
        }
        parse_evidence_instant(
            "company_financials.projection",
            evidence.provider,
            "observed_at",
            &evidence.observed_at,
        )
        .map_err(|error| anyhow!("BR-158 financial projection evidence is invalid: {error}"))?;
        if evidence
            .source_at
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(anyhow!(
                "BR-158 financial projection source_at is present but empty"
            ));
        }
        if self.source.as_deref() != Some(evidence.source.as_str()) {
            return Err(anyhow!(
                "BR-158 financial projection source differs from admitted evidence"
            ));
        }
        Ok(evidence)
    }
}

/// Conservatively project the legacy analysis view from one complete admitted
/// income-statement batch.
///
/// Sina's normalized financial-statement contract currently exposes a stable
/// basic-EPS line, but does not expose the old F10 ratio/growth fields with the
/// same semantics. Those fields intentionally remain `None`.
pub fn project_income_statements(batch: GatewayBatch<FinancialStatement>) -> Result<Financials> {
    let evidence = batch.evidence().clone();
    if evidence.source.trim().is_empty()
        || evidence.observed_at.trim().is_empty()
        || evidence.batch_id.trim().is_empty()
        || evidence
            .source_at
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
    {
        return Err(anyhow!(
            "BR-158 admitted financial batch evidence is incomplete"
        ));
    }
    parse_evidence_instant(
        "company_financials.income_statements",
        evidence.provider,
        "observed_at",
        &evidence.observed_at,
    )
    .map_err(|error| anyhow!("BR-158 admitted financial batch evidence is invalid: {error}"))?;
    let records = match batch {
        GatewayBatch::Available { records, .. } if !records.is_empty() => records,
        GatewayBatch::Available { .. } | GatewayBatch::VerifiedEmpty(_) => {
            return Err(anyhow!(
                "BR-164 company income statement batch is verified empty"
            ))
        }
    };
    let instrument = &records[0].instrument;
    if records.iter().any(|statement| {
        &statement.instrument != instrument || statement.kind != StatementKind::Income
    }) {
        return Err(anyhow!(
            "BR-164 income-statement projection requires one instrument and Income records only"
        ));
    }
    for statement in &records {
        if statement.evidence.provider() != evidence.provider
            || statement.evidence.batch_id() != evidence.batch_id
            || statement.evidence.observed_at() != evidence.observed_at
        {
            return Err(anyhow!(
                "BR-158 financial record evidence mismatch for {} {}",
                statement.instrument.code(),
                statement.report_period
            ));
        }
        let announced_on = statement.announced_on.as_ref().map(|date| date.as_str());
        if statement.evidence.source_at() != announced_on {
            return Err(anyhow!(
                "BR-158 financial record provider time mismatch for {} {}",
                statement.instrument.code(),
                statement.report_period
            ));
        }
    }
    let latest_record_source_at = records
        .iter()
        .filter_map(|statement| statement.announced_on.as_ref().map(|date| date.as_str()))
        .max();
    if evidence.source_at.as_deref() != latest_record_source_at {
        return Err(anyhow!(
            "BR-158 financial batch provider time differs from retained records"
        ));
    }

    let mut history = Vec::with_capacity(records.len());
    for statement in &records {
        let eps = statement.lines.iter().find_map(|line| {
            let key = line.key.as_str();
            let label = line.source_label.as_str();
            ((key == "basiceps" || label == "基本每股收益")
                && line.unit.as_ref().is_none_or(|unit| unit.as_str() == "元"))
            .then(|| line.value.map(|value| value.get()))
            .flatten()
        });
        let period = FinancialPeriod {
            report_date: Some(statement.report_period.as_str().to_string()),
            eps,
            ..FinancialPeriod::default()
        };
        if period.any() {
            history.push(period);
        }
    }
    let latest = history.first().ok_or_else(|| {
        anyhow!("BR-164 admitted income statements contain no exactly projected financial metric")
    })?;
    let newest_statement = records
        .first()
        .ok_or_else(|| anyhow!("BR-164 admitted income statement batch has no newest statement"))?;
    let published_date = newest_statement
        .announced_on
        .as_ref()
        .map(|date| date.as_str().to_string());
    let canonical_records = serde_json::to_vec(&records)
        .map_err(|error| anyhow!("BR-158 financial content serialization failed: {error}"))?;
    let mut content_hasher = Sha256::new();
    content_hasher.update(b"stock_analysis.financial_projection_content.v1\0");
    content_hasher.update(&canonical_records);
    let projection_evidence = FinancialProjectionEvidence {
        provider: evidence.provider,
        source: evidence.source.clone(),
        source_at: evidence.source_at.clone(),
        observed_at: evidence.observed_at.clone(),
        batch_id: evidence.batch_id.clone(),
        content_sha256: format!("{:x}", content_hasher.finalize()),
    };

    let projected = Financials {
        report_date: latest.report_date.clone(),
        published_date,
        eps: latest.eps,
        source: Some(evidence.source),
        evidence: Some(projection_evidence),
        history,
        ..Financials::default()
    };
    projected.require_projection_evidence()?;
    Ok(projected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::BatchEvidence;
    use magic_market_core::{
        AssetClass, Exchange, FiniteNumber, InstrumentId, IsoDate, NonEmptyText, ProviderId,
        SourceEvidence,
    };

    #[test]
    fn quality_requires_history() {
        assert!(assess_quality(&[]).is_none());
    }

    #[test]
    fn projection_keeps_only_exact_basic_eps_and_explicitly_missing_ratios() {
        let instrument =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600519", AssetClass::Equity)
                .expect("test instrument");
        let statement = FinancialStatement {
            instrument,
            kind: StatementKind::Income,
            report_period: IsoDate::new("2026-03-31").expect("report date"),
            announced_on: Some(IsoDate::new("2026-04-30").expect("announcement date")),
            currency: Some(NonEmptyText::new("CNY").expect("currency")),
            lines: vec![
                magic_market_core::FinancialLine {
                    key: NonEmptyText::new("basiceps").expect("key"),
                    source_label: NonEmptyText::new("基本每股收益").expect("label"),
                    value: Some(FiniteNumber::new(1.23).expect("finite EPS")),
                    unit: None,
                },
                magic_market_core::FinancialLine {
                    key: NonEmptyText::new("bizinco").expect("key"),
                    source_label: NonEmptyText::new("营业收入").expect("label"),
                    value: Some(FiniteNumber::new(100.0).expect("finite revenue")),
                    unit: None,
                },
            ],
            evidence: SourceEvidence::new(
                ProviderId::Sina,
                "1785799979.851045000",
                "TEST_CODE_BATCH",
            )
            .expect("evidence")
            .with_source_at("2026-04-30")
            .expect("source date"),
        };
        let batch = GatewayBatch::Available {
            records: vec![statement],
            evidence: BatchEvidence {
                provider: ProviderId::Sina,
                source: "TEST_CODE_SINA".into(),
                source_at: Some("2026-04-30".into()),
                observed_at: "1785799979.851045000".into(),
                batch_id: "TEST_CODE_BATCH".into(),
            },
        };

        let projected = project_income_statements(batch).expect("exact projection");
        assert_eq!(projected.report_date.as_deref(), Some("2026-03-31"));
        assert_eq!(projected.published_date.as_deref(), Some("2026-04-30"));
        assert_eq!(projected.eps, Some(1.23));
        assert_eq!(projected.revenue_yoy, None);
        assert_eq!(projected.net_profit_yoy, None);
        assert_eq!(projected.gross_margin, None);
        assert_eq!(projected.roe, None);
        let evidence = projected
            .evidence
            .as_ref()
            .expect("admitted projection must retain batch evidence");
        assert_eq!(evidence.provider, ProviderId::Sina);
        assert_eq!(evidence.source, "TEST_CODE_SINA");
        assert_eq!(evidence.source_at.as_deref(), Some("2026-04-30"));
        assert_eq!(evidence.observed_at, "1785799979.851045000");
        assert_eq!(evidence.batch_id, "TEST_CODE_BATCH");
        assert_eq!(evidence.content_sha256.len(), 64);
        assert_eq!(projected.source.as_deref(), Some("TEST_CODE_SINA"));
    }

    #[test]
    fn projection_rejects_record_batch_evidence_mismatch() {
        let instrument =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600519", AssetClass::Equity)
                .expect("test instrument");
        let statement = FinancialStatement {
            instrument,
            kind: StatementKind::Income,
            report_period: IsoDate::new("2026-03-31").expect("report date"),
            announced_on: Some(IsoDate::new("2026-04-30").expect("announcement date")),
            currency: Some(NonEmptyText::new("CNY").expect("currency")),
            lines: vec![magic_market_core::FinancialLine {
                key: NonEmptyText::new("basiceps").expect("key"),
                source_label: NonEmptyText::new("基本每股收益").expect("label"),
                value: Some(FiniteNumber::new(1.23).expect("finite EPS")),
                unit: None,
            }],
            evidence: SourceEvidence::new(
                ProviderId::Sina,
                "2026-04-30T10:00:00+08:00",
                "TEST_CODE_OTHER_BATCH",
            )
            .expect("evidence")
            .with_source_at("2026-04-30")
            .expect("source date"),
        };
        let batch = GatewayBatch::Available {
            records: vec![statement],
            evidence: BatchEvidence {
                provider: ProviderId::Sina,
                source: "TEST_CODE_SINA".into(),
                source_at: Some("2026-04-30".into()),
                observed_at: "2026-04-30T10:00:00+08:00".into(),
                batch_id: "TEST_CODE_BATCH".into(),
            },
        };

        let error = project_income_statements(batch).expect_err("mismatched batch must fail");
        assert!(error.to_string().contains("evidence mismatch"));
    }
}
