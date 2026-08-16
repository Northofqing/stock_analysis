use crate::data_gateway::{
    EconomicReleaseFact, GatewayBatch, GatewayError, GlobalNewsProvider, GlobalNewsRecord,
};

const MAX_NEWS_PER_SOURCE: usize = 8;
const MIN_RELEASE_IMPORTANCE: u32 = 2;
const MAX_RELEASES: usize = 15;

type GlobalNewsOutcome = Result<GatewayBatch<GlobalNewsRecord>, GatewayError>;
type EconomicReleaseOutcome = Result<GatewayBatch<EconomicReleaseFact>, GatewayError>;

pub(crate) fn render_gateway_sections(
    news_outcomes: [(GlobalNewsProvider, GlobalNewsOutcome); 4],
    release_outcome: EconomicReleaseOutcome,
) -> Vec<String> {
    let mut sections = news_outcomes
        .into_iter()
        .map(|(provider, outcome)| render_news_section(provider, outcome))
        .collect::<Vec<_>>();
    sections.push(render_release_section(release_outcome));
    sections
}

fn render_news_section(provider: GlobalNewsProvider, outcome: GlobalNewsOutcome) -> String {
    let header = news_header(provider);
    match outcome {
        Ok(batch) if batch.is_verified_empty() => format!(
            "{header}\n- 已验证无最新新闻（source={} batch_id={}）",
            batch.evidence().source,
            batch.evidence().batch_id
        ),
        Ok(batch) => {
            let evidence = batch.evidence();
            let mut lines = vec![format!(
                "_source={} source_at={} batch_id={}_",
                evidence.source,
                evidence.source_at.as_deref().unwrap_or("缺失"),
                evidence.batch_id
            )];
            for record in batch.records().iter().take(MAX_NEWS_PER_SOURCE) {
                lines.push(format!(
                    "- **{}** `{}`（{}）",
                    record.title,
                    record.published_at.to_rfc3339(),
                    record.publisher
                ));
                if let Some(detail) = record.summary.as_deref().or(record.content.as_deref()) {
                    lines.push(format!(
                        "  {}",
                        detail.chars().take(180).collect::<String>()
                    ));
                }
            }
            format!("{header}\n{}", lines.join("\n"))
        }
        Err(error) => format!(
            "{header}\n- 数据不可用：reason_code={} retryable={}",
            error.reason_code(),
            error.retryable()
        ),
    }
}

fn render_release_section(outcome: EconomicReleaseOutcome) -> String {
    const HEADER: &str = "### 📊 最新经济数据发布（金十）";
    match outcome {
        Ok(batch) => {
            let evidence = batch.evidence();
            let mut lines = vec![format!(
                "_source={} source_at={} batch_id={}_",
                evidence.source,
                evidence.source_at.as_deref().unwrap_or("缺失"),
                evidence.batch_id
            )];
            let releases = batch
                .records()
                .iter()
                .filter(|release| release.importance >= MIN_RELEASE_IMPORTANCE)
                .take(MAX_RELEASES)
                .collect::<Vec<_>>();
            if releases.is_empty() {
                lines.push(format!(
                    "- 已验证：完整批次无 importance>={MIN_RELEASE_IMPORTANCE} 的最新发布"
                ));
            } else {
                for release in releases {
                    let stars = "★".repeat(release.importance.min(5) as usize);
                    lines.push(format!(
                        "- `{}` {} **[{}]** {}",
                        release.released_at.to_rfc3339(),
                        stars,
                        release.country,
                        release.name
                    ));
                    let mut facts = Vec::new();
                    push_fact(&mut facts, "周期", release.period.as_deref());
                    push_fact(&mut facts, "前值", release.previous.as_deref());
                    push_fact(&mut facts, "预期", release.consensus.as_deref());
                    push_fact(&mut facts, "公布", release.actual.as_deref());
                    push_fact(&mut facts, "修正", release.revised.as_deref());
                    push_fact(&mut facts, "单位", release.unit.as_deref());
                    push_fact(&mut facts, "影响", release.impact.as_deref());
                    if !facts.is_empty() {
                        lines.push(format!("  {}", facts.join(" | ")));
                    }
                }
            }
            format!("{HEADER}\n{}", lines.join("\n"))
        }
        Err(error) => format!(
            "{HEADER}\n- 数据不可用：reason_code={} retryable={}",
            error.reason_code(),
            error.retryable()
        ),
    }
}

fn push_fact(facts: &mut Vec<String>, label: &str, value: Option<&str>) {
    if let Some(value) = value {
        facts.push(format!("{label} {value}"));
    }
}

const fn news_header(provider: GlobalNewsProvider) -> &'static str {
    match provider {
        GlobalNewsProvider::Eastmoney => "### 📰 东方财富财经要闻",
        GlobalNewsProvider::Cailianpress => "### 🧭 财联社电报",
        GlobalNewsProvider::Jin10 => "### 📣 金十快讯",
        GlobalNewsProvider::ThePaper => "### 🌐 澎湃财经",
    }
}

#[cfg(test)]
mod tests {
    use super::render_gateway_sections;
    use crate::data_gateway::{
        BatchEvidence, EconomicReleaseFact, GatewayBatch, GatewayError, GlobalNewsProvider,
        GlobalNewsRecord,
    };
    use crate::magic_compat::{ProviderId, SourceEvidence};
    use chrono::{DateTime, Utc};

    fn timestamp(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("TEST_CODE timestamp")
            .with_timezone(&Utc)
    }

    fn evidence(provider: ProviderId, source: &str, batch_id: &str) -> BatchEvidence {
        BatchEvidence {
            provider,
            source: source.to_owned(),
            source_at: Some("2026-07-25 10:00:00".to_owned()),
            observed_at: "1784959200.000000000".to_owned(),
            batch_id: batch_id.to_owned(),
        }
    }

    fn source_evidence(provider: ProviderId, batch_id: &str, source_at: &str) -> SourceEvidence {
        SourceEvidence::new(provider, "1784959200.000000000", batch_id)
            .and_then(|value| value.with_source_at(source_at))
            .expect("TEST_CODE source evidence")
    }

    fn news_record(provider: ProviderId, batch_id: &str, title: &str) -> GlobalNewsRecord {
        GlobalNewsRecord {
            item_id: format!("TEST_CODE_{batch_id}"),
            title: title.to_owned(),
            summary: None,
            content: None,
            publisher: "TEST_CODE publisher".to_owned(),
            canonical_url: format!("https://example.invalid/{batch_id}"),
            published_at: timestamp("2026-07-25T02:00:00Z"),
            observed_at: timestamp("2026-07-25T02:00:01Z"),
            instruments: Vec::new(),
            topics: Vec::new(),
            language: "zh-CN".to_owned(),
            evidence: source_evidence(provider, batch_id, "2026-07-25 10:00:00"),
        }
    }

    fn release(event_id: &str, importance: u32, name: &str) -> EconomicReleaseFact {
        EconomicReleaseFact {
            event_id: event_id.to_owned(),
            indicator_id: 1,
            country: "TEST_CODE 中国".to_owned(),
            name: name.to_owned(),
            period: None,
            scheduled_at: timestamp("2026-07-25T02:00:00Z"),
            released_at: timestamp("2026-07-25T02:00:00Z"),
            previous: Some("1.0".to_owned()),
            consensus: None,
            actual: None,
            revised: None,
            unit: None,
            importance,
            impact: None,
            evidence: source_evidence(
                ProviderId::Jin10,
                "TEST_CODE_economic",
                "2026-07-25 10:00:00",
            ),
        }
    }

    #[test]
    fn renders_independent_gateway_outcomes_and_latest_releases() {
        let sections = render_gateway_sections(
            [
                (
                    GlobalNewsProvider::Eastmoney,
                    Ok(GatewayBatch::Available {
                        records: vec![news_record(
                            ProviderId::Eastmoney,
                            "TEST_CODE_eastmoney",
                            "TEST_CODE 东方财富新闻",
                        )],
                        evidence: evidence(
                            ProviderId::Eastmoney,
                            "eastmoney-web",
                            "TEST_CODE_eastmoney",
                        ),
                    }),
                ),
                (
                    GlobalNewsProvider::Cailianpress,
                    Ok(GatewayBatch::VerifiedEmpty(evidence(
                        ProviderId::Cailianpress,
                        "cls-v1",
                        "TEST_CODE_cls",
                    ))),
                ),
                (
                    GlobalNewsProvider::Jin10,
                    Err(GatewayError::unavailable(
                        "TEST_CODE_GlobalNews-Jin10",
                        Some(ProviderId::Jin10),
                        true,
                        "TEST_CODE transport unavailable",
                    )),
                ),
                (
                    GlobalNewsProvider::ThePaper,
                    Ok(GatewayBatch::Available {
                        records: vec![news_record(
                            ProviderId::ThePaper,
                            "TEST_CODE_thepaper",
                            "TEST_CODE 澎湃新闻",
                        )],
                        evidence: evidence(
                            ProviderId::ThePaper,
                            "thepaper-finance-v1",
                            "TEST_CODE_thepaper",
                        ),
                    }),
                ),
            ],
            Ok(GatewayBatch::Available {
                records: vec![
                    release("TEST_CODE_high", 3, "TEST_CODE 高重要性指标"),
                    release("TEST_CODE_low", 1, "TEST_CODE 低重要性指标"),
                ],
                evidence: evidence(ProviderId::Jin10, "jin10-flash-v1", "TEST_CODE_economic"),
            }),
        );
        let report = sections.join("\n\n");

        assert_eq!(sections.len(), 5);
        assert!(report.contains("TEST_CODE 东方财富新闻"));
        assert!(report.contains("财联社"));
        assert!(report.contains("已验证无最新新闻"));
        assert!(report.contains("金十"));
        assert!(report.contains("不可用"));
        assert!(report.contains("reason_code=no_verified_batch"));
        assert!(report.contains("TEST_CODE 澎湃新闻"));
        assert!(report.contains("最新经济数据发布"));
        assert!(report.contains("TEST_CODE 高重要性指标"));
        assert!(!report.contains("TEST_CODE 低重要性指标"));
        assert!(!report.contains("未来48h"));
    }
}
