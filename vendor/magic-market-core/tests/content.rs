use magic_market_core::{
    Announcement, AssetClass, Exchange, HttpsUrl, InstrumentId, InvestorQuestion, NewsItem,
    NonEmptyText, ProviderId, SourceEvidence, SourcedRecord,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity).unwrap()
}

fn evidence(provider: ProviderId) -> SourceEvidence {
    SourceEvidence::new(provider, "observed", "batch").unwrap()
}

#[test]
fn news_and_announcement_urls_are_https_and_sourced() {
    let news = NewsItem {
        item_id: NonEmptyText::new("news-1").unwrap(),
        title: NonEmptyText::new("华电辽能新闻").unwrap(),
        summary: None,
        content: None,
        publisher: NonEmptyText::new("东方财富").unwrap(),
        canonical_url: HttpsUrl::new("https://example.com/news-1").unwrap(),
        published_at: NonEmptyText::new("2026-07-23 10:00:00").unwrap(),
        instruments: vec![instrument()],
        topics: vec![],
        language: NonEmptyText::new("zh-CN").unwrap(),
        evidence: evidence(ProviderId::Eastmoney),
    };
    let announcement = Announcement {
        announcement_id: NonEmptyText::new("ann-1").unwrap(),
        instrument: instrument(),
        instrument_name: Some(NonEmptyText::new("华电辽能").unwrap()),
        category: Some(NonEmptyText::new("公司公告").unwrap()),
        title: NonEmptyText::new("年度报告").unwrap(),
        published_at: NonEmptyText::new("2026-07-23").unwrap(),
        canonical_url: HttpsUrl::new("https://example.com/ann-1").unwrap(),
        pdf_url: None,
        evidence: evidence(ProviderId::Cninfo),
    };

    assert_eq!(news.provider_id(), ProviderId::Eastmoney);
    assert_eq!(news.evidence_batch_id(), "batch");
    assert_eq!(announcement.provider_id(), ProviderId::Cninfo);
    assert_eq!(announcement.evidence_batch_id(), "batch");
    assert!(serde_json::from_str::<NewsItem>(
        &serde_json::to_string(&news)
            .unwrap()
            .replace("https://example.com", "http://example.com")
    )
    .is_err());
}

#[test]
fn unanswered_investor_question_is_not_fabricated() {
    let question = InvestorQuestion::new(
        NonEmptyText::new("q-1").unwrap(),
        instrument(),
        NonEmptyText::new("公司").unwrap(),
        NonEmptyText::new("请问项目进展？").unwrap(),
        NonEmptyText::new("2026-07-23 09:00:00").unwrap(),
        None,
        None,
        evidence(ProviderId::Cninfo),
    )
    .unwrap();
    assert_eq!(question.question_id().as_str(), "q-1");
    assert_eq!(question.instrument(), &instrument());
    assert_eq!(question.company().as_str(), "公司");
    assert_eq!(question.question().as_str(), "请问项目进展？");
    assert_eq!(question.question_at().as_str(), "2026-07-23 09:00:00");
    assert!(question.answer().is_none());
    assert!(question.answer_at().is_none());
    assert!(question.source_question_id().is_none());
    assert!(question.answerer().is_none());
    assert_eq!(question.evidence().provider(), ProviderId::Cninfo);
    assert_eq!(question.provider_id(), ProviderId::Cninfo);
    assert_eq!(question.evidence_batch_id(), "batch");
    assert_eq!(
        serde_json::from_value::<InvestorQuestion>(serde_json::to_value(&question).unwrap())
            .unwrap(),
        question
    );

    assert!(InvestorQuestion::new(
        NonEmptyText::new("q-2").unwrap(),
        instrument(),
        NonEmptyText::new("公司").unwrap(),
        NonEmptyText::new("问题").unwrap(),
        NonEmptyText::new("2026-07-23").unwrap(),
        None,
        Some(NonEmptyText::new("2026-07-24").unwrap()),
        evidence(ProviderId::Cninfo),
    )
    .is_err());
    assert!(InvestorQuestion::new_with_metadata(
        NonEmptyText::new("q-3").unwrap(),
        instrument(),
        NonEmptyText::new("公司").unwrap(),
        NonEmptyText::new("问题").unwrap(),
        NonEmptyText::new("2026-07-23").unwrap(),
        None,
        None,
        Some(NonEmptyText::new("source-3").unwrap()),
        Some(NonEmptyText::new("董秘").unwrap()),
        evidence(ProviderId::Cninfo),
    )
    .is_err());
}
