use super::*;

#[test]
fn policy_request_requires_a_complete_ordered_range() {
    let invalid = r#"{"query":null,"start":"2026-07-01","end":null,"page":1,"page_size":5}"#;
    assert!(serde_json::from_str::<PolicyRequest>(invalid).is_err());
    let request =
        PolicyRequest::new(PositiveU32::new(1).unwrap(), PositiveU32::new(5).unwrap()).unwrap();
    assert!(request
        .clone()
        .with_range(
            IsoDate::new("2026-07-02").unwrap(),
            IsoDate::new("2026-07-01").unwrap()
        )
        .is_err());
}

#[test]
fn policy_request_round_trips_query_range_and_pagination() {
    let request = PolicyRequest::new(PositiveU32::new(2).unwrap(), PositiveU32::new(50).unwrap())
        .unwrap()
        .with_query("产业政策")
        .unwrap()
        .with_range(
            IsoDate::new("2026-07-01").unwrap(),
            IsoDate::new("2026-07-25").unwrap(),
        )
        .unwrap();
    assert_eq!(request.query().unwrap().as_str(), "产业政策");
    assert_eq!(request.start().unwrap().as_str(), "2026-07-01");
    assert_eq!(request.end().unwrap().as_str(), "2026-07-25");
    assert_eq!(request.page().get(), 2);
    assert_eq!(request.page_size().get(), 50);

    let restored: PolicyRequest = serde_json::from_str(
        r#"{"query":"产业政策","start":"2026-07-01","end":"2026-07-25","page":2,"page_size":50}"#,
    )
    .unwrap();
    assert_eq!(restored, request);

    let unfiltered: PolicyRequest =
        serde_json::from_str(r#"{"query":null,"start":null,"end":null,"page":1,"page_size":5}"#)
            .unwrap();
    assert!(unfiltered.query().is_none());
    assert!(unfiltered.start().is_none());
    assert!(unfiltered.end().is_none());
}

#[test]
fn policy_request_rejects_invalid_page_query_and_partial_ranges() {
    assert!(
        PolicyRequest::new(PositiveU32::new(1).unwrap(), PositiveU32::new(51).unwrap()).is_err()
    );
    assert!(
        PolicyRequest::new(PositiveU32::new(1).unwrap(), PositiveU32::new(5).unwrap())
            .unwrap()
            .with_query("   ")
            .is_err()
    );
    assert!(serde_json::from_str::<PolicyRequest>(
        r#"{"query":null,"start":null,"end":"2026-07-25","page":1,"page_size":5}"#
    )
    .is_err());
}

#[test]
fn policy_document_exposes_source_identity() {
    let document = PolicyDocument {
        document_id: NonEmptyText::new("policy-1").unwrap(),
        title: NonEmptyText::new("产业政策").unwrap(),
        summary: None,
        organization: NonEmptyText::new("国务院").unwrap(),
        document_number: None,
        category: None,
        published_date: IsoDate::new("2026-07-25").unwrap(),
        canonical_url: HttpsUrl::new("https://www.gov.cn/policy.html").unwrap(),
        evidence: SourceEvidence::new(crate::ProviderId::StateCouncil, "observed", "policy-batch")
            .unwrap(),
    };
    assert_eq!(document.provider_id(), crate::ProviderId::StateCouncil);
    assert_eq!(document.evidence_batch_id(), "policy-batch");
}
