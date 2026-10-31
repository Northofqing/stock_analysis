use magic_market_core::{
    AssetClass, ConsensusSnapshot, EarningsEstimate, Exchange, FiniteNumber, HttpsUrl,
    InstrumentId, Money, NonEmptyText, PositiveU32, ProviderId, ReportScope, ResearchReport,
    ResearchRequest, SemanticChannel, SemanticSearchDocument, SemanticSearchRequest,
    SourceEvidence, SourcedRecord,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity).unwrap()
}

fn evidence(provider: ProviderId) -> SourceEvidence {
    SourceEvidence::new(provider, "observed", "batch").unwrap()
}

#[test]
fn research_and_consensus_records_are_typed_and_sourced() {
    let estimate = EarningsEstimate::new(
        PositiveU32::new(2026).unwrap(),
        Some(FiniteNumber::new(0.42).unwrap()),
        Some(FiniteNumber::new(0.40).unwrap()),
        Some(FiniteNumber::new(0.44).unwrap()),
        Some(PositiveU32::new(5).unwrap()),
        None,
        None,
    )
    .unwrap();
    let report = ResearchReport {
        report_id: NonEmptyText::new("H3_ABC").unwrap(),
        scope: ReportScope::Instrument(instrument()),
        title: NonEmptyText::new("公司深度报告").unwrap(),
        organization: NonEmptyText::new("示例证券").unwrap(),
        organization_id: None,
        author: None,
        rating: Some(NonEmptyText::new("增持").unwrap()),
        industry_code: None,
        industry_name: None,
        published_at: NonEmptyText::new("2026-07-23 08:00:00").unwrap(),
        canonical_url: HttpsUrl::new("https://example.com/report/H3_ABC").unwrap(),
        pdf_url: Some(HttpsUrl::new("https://example.com/H3_ABC.pdf").unwrap()),
        estimates: vec![estimate.clone()],
        source_indv_aim_price_t: None,
        source_indv_aim_price_l: None,
        evidence: evidence(ProviderId::Eastmoney),
    };
    let consensus = ConsensusSnapshot {
        instrument: instrument(),
        name: NonEmptyText::new("华电辽能").unwrap(),
        estimates: vec![estimate],
        contributor_count: Some(PositiveU32::new(5).unwrap()),
        evidence: evidence(ProviderId::Tonghuashun),
    };

    assert_eq!(report.provider_id(), ProviderId::Eastmoney);
    assert_eq!(report.evidence_batch_id(), "batch");
    assert_eq!(consensus.provider_id(), ProviderId::Tonghuashun);
    assert_eq!(consensus.name.as_str(), "华电辽能");
    assert_eq!(consensus.evidence_batch_id(), "batch");
    let json = serde_json::to_string(&report).unwrap();
    assert_eq!(
        serde_json::from_str::<ResearchReport>(&json).unwrap(),
        report
    );
    assert!(serde_json::from_str::<EarningsEstimate>(
        r#"{"fiscal_year":2026,"eps":0.42,"eps_min":0.5,"eps_max":0.4,"contributor_count":5,"revenue":null,"profit":null}"#
    )
    .is_err());
    assert!(EarningsEstimate::new(
        PositiveU32::new(2026).unwrap(),
        Some(FiniteNumber::new(0.42).unwrap()),
        Some(FiniteNumber::new(0.5).unwrap()),
        Some(FiniteNumber::new(0.4).unwrap()),
        None,
        None,
        None,
    )
    .is_err());
}

#[test]
fn earnings_estimate_accessors_preserve_source_values() {
    let estimate = EarningsEstimate::new(
        PositiveU32::new(2027).unwrap(),
        Some(FiniteNumber::new(0.52).unwrap()),
        Some(FiniteNumber::new(0.50).unwrap()),
        Some(FiniteNumber::new(0.54).unwrap()),
        Some(PositiveU32::new(8).unwrap()),
        Some(Money::new(2_000_000_000.0).unwrap()),
        Some(Money::new(300_000_000.0).unwrap()),
    )
    .unwrap();

    assert_eq!(estimate.fiscal_year().get(), 2027);
    assert_eq!(estimate.eps().unwrap().get(), 0.52);
    assert_eq!(estimate.eps_min().unwrap().get(), 0.50);
    assert_eq!(estimate.eps_max().unwrap().get(), 0.54);
    assert_eq!(estimate.contributor_count().unwrap().get(), 8);
    assert_eq!(estimate.revenue().unwrap().get(), 2_000_000_000.0);
    assert_eq!(estimate.profit().unwrap().get(), 300_000_000.0);
}

#[test]
fn research_requests_are_bounded_and_checked_during_deserialization() {
    let request = ResearchRequest::new(
        ReportScope::Instrument(instrument()),
        PositiveU32::new(2).unwrap(),
        PositiveU32::new(50).unwrap(),
    )
    .unwrap();
    assert_eq!(request.scope(), &ReportScope::Instrument(instrument()));
    assert_eq!(request.page().get(), 2);
    assert_eq!(request.page_size().get(), 50);
    assert_eq!(
        serde_json::from_value::<ResearchRequest>(serde_json::to_value(&request).unwrap()).unwrap(),
        request
    );
    assert!(ResearchRequest::new(
        ReportScope::Industry(NonEmptyText::new("电力").unwrap()),
        PositiveU32::new(1).unwrap(),
        PositiveU32::new(101).unwrap(),
    )
    .is_err());
    assert!(
        serde_json::from_value::<ResearchRequest>(serde_json::json!({
            "scope": {"Industry": "电力"},
            "page": 1,
            "page_size": 101
        }))
        .is_err()
    );
}

#[test]
fn semantic_search_is_bounded_and_uses_https_documents() {
    let request = SemanticSearchRequest::new(
        "华电辽能 研报",
        SemanticChannel::Report,
        PositiveU32::new(20).unwrap(),
    )
    .unwrap();
    assert_eq!(request.query().as_str(), "华电辽能 研报");
    assert_eq!(request.channel(), SemanticChannel::Report);
    assert_eq!(request.limit().get(), 20);
    assert_eq!(
        serde_json::from_value::<SemanticSearchRequest>(serde_json::to_value(&request).unwrap())
            .unwrap(),
        request
    );
    assert!(SemanticSearchRequest::new(
        "query",
        SemanticChannel::General,
        PositiveU32::new(101).unwrap()
    )
    .is_err());

    let document = SemanticSearchDocument {
        document_id: NonEmptyText::new("doc-1").unwrap(),
        channel: SemanticChannel::Report,
        title: NonEmptyText::new("报告").unwrap(),
        excerpt: None,
        canonical_url: HttpsUrl::new("https://example.com/doc-1").unwrap(),
        published_at: None,
        evidence: evidence(ProviderId::Iwencai),
    };
    assert_eq!(document.provider_id(), ProviderId::Iwencai);
    assert_eq!(document.evidence_batch_id(), "batch");
    assert!(serde_json::from_str::<SemanticSearchDocument>(
        r#"{"document_id":"d","channel":"Report","title":"t","excerpt":null,"canonical_url":"http://example.com","published_at":null,"evidence":{"provider":"Iwencai","source_at":null,"observed_at":"o","batch_id":"b"}}"#
    )
    .is_err());
}
