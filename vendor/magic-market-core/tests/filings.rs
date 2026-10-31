use magic_market_core::{
    CompanyFiling, CompanyFilingRequest, HttpsUrl, IsoDate, NonEmptyText, PositiveU32, ProviderId,
    SecAccessionNumber, SecCompanyIdentity, SecPrimaryDocument, SourceEvidence, SourcedRecord,
};

#[test]
fn cik_is_normalized_and_company_requests_are_bounded() {
    let company = SecCompanyIdentity::new("320193", Some("AAPL")).unwrap();
    assert_eq!(company.cik(), "0000320193");
    assert_eq!(company.ticker(), Some("AAPL"));
    assert!(SecCompanyIdentity::new("12345678901", None::<String>).is_err());
    assert!(CompanyFilingRequest::new(
        vec![company.clone(), company],
        vec![],
        None,
        None,
        PositiveU32::new(10).unwrap(),
    )
    .is_err());
}

#[test]
fn accession_and_primary_document_are_path_safe() {
    assert!(SecAccessionNumber::new("0000320193-25-000079").is_ok());
    assert!(SecAccessionNumber::new("../000079").is_err());
    assert!(SecPrimaryDocument::new("../report.htm").is_err());
    for unsafe_name in [
        "%2e%2e%2freport.htm",
        "%2Freport.htm",
        "report%5Cevil.htm",
        "report final.htm",
    ] {
        assert!(SecPrimaryDocument::new(unsafe_name).is_err());
    }
}

#[test]
fn serde_cannot_bypass_filing_invariants() {
    assert!(serde_json::from_str::<SecAccessionNumber>("\"../000079\"").is_err());
    assert!(serde_json::from_str::<SecPrimaryDocument>("\"../report.htm\"").is_err());

    let reversed = r#"{
      "companies":[{"cik":"320193","ticker":"AAPL"}],"forms":[],
      "start":"2026-07-29","end":"2026-07-01","max_records":10
    }"#;
    assert!(serde_json::from_str::<CompanyFilingRequest>(reversed).is_err());

    let oversized = r#"{
      "companies":[{"cik":"320193","ticker":"AAPL"}],"forms":[],
      "start":null,"end":null,"max_records":1001
    }"#;
    assert!(serde_json::from_str::<CompanyFilingRequest>(oversized).is_err());

    let timestamp_disagreement = r#"{
      "company":{"cik":"320193","ticker":"AAPL"},
      "company_name":"Apple Inc.","form":"10-K","filing_date":"2026-07-29",
      "report_period":null,"accession":"0000320193-25-000079",
      "primary_document":"report.htm",
      "filing_index_url":"https://www.sec.gov/Archives/example-index.htm",
      "primary_document_url":"https://www.sec.gov/Archives/report.htm",
      "accepted_at":"2026-07-29T08:00:00Z",
      "evidence":{"provider":"SecEdgar",
        "source_at":"2026-07-29T09:00:00Z",
        "observed_at":"2026-07-29T10:00:00Z","batch_id":"sec-1"}
    }"#;
    assert!(serde_json::from_str::<CompanyFiling>(timestamp_disagreement).is_err());
}

#[test]
fn company_filing_exposes_all_source_evidence() {
    let accepted_at = "2026-07-29T09:00:00Z";
    let observed_at = "2026-07-29T10:00:00Z";
    let evidence = SourceEvidence::new(ProviderId::SecEdgar, observed_at, "sec-evidence")
        .unwrap()
        .with_source_at(accepted_at)
        .unwrap();
    let filing = CompanyFiling::new(
        SecCompanyIdentity::new("320193", Some("AAPL")).unwrap(),
        "Apple Inc.",
        "10-K",
        IsoDate::new("2026-07-29").unwrap(),
        None,
        SecAccessionNumber::new("0000320193-25-000079").unwrap(),
        SecPrimaryDocument::new("report.htm").unwrap(),
        HttpsUrl::new("https://www.sec.gov/Archives/example-index.htm").unwrap(),
        HttpsUrl::new("https://www.sec.gov/Archives/report.htm").unwrap(),
        Some(NonEmptyText::new(accepted_at).unwrap()),
        evidence,
    )
    .unwrap();
    assert_eq!(filing.provider_id(), ProviderId::SecEdgar);
    assert_eq!(filing.evidence_batch_id(), "sec-evidence");
    assert_eq!(filing.evidence_source_at(), Some(accepted_at));
    assert_eq!(filing.evidence_observed_at(), Some(observed_at));
    assert_eq!(filing.company_name(), "Apple Inc.");
    assert_eq!(filing.primary_document().as_str(), "report.htm");
    assert_eq!(filing.accession().to_string(), "0000320193-25-000079");
    assert_eq!(filing.primary_document().to_string(), "report.htm");
}

#[test]
fn ticker_and_request_cardinality_filters_fail_closed() {
    for ticker in ["", "TOO-LONG-123", "BAD_TICKER", "坏"] {
        assert!(
            SecCompanyIdentity::new("320193", Some(ticker)).is_err(),
            "{ticker:?}"
        );
    }
    let company = SecCompanyIdentity::new("320193", Some("AAPL")).unwrap();
    let limit = PositiveU32::new(10).unwrap();
    assert!(CompanyFilingRequest::new(vec![], vec![], None, None, limit).is_err());
    let companies = (1..=101)
        .map(|cik| SecCompanyIdentity::new(cik.to_string(), None::<String>).unwrap())
        .collect();
    assert!(CompanyFilingRequest::new(companies, vec![], None, None, limit).is_err());
    let forms = (0..21)
        .map(|index| NonEmptyText::new(format!("FORM-{index}")).unwrap())
        .collect();
    assert!(CompanyFilingRequest::new(vec![company.clone()], forms, None, None, limit).is_err());
    let form = NonEmptyText::new("10-K").unwrap();
    assert!(CompanyFilingRequest::new(
        vec![company.clone()],
        vec![form.clone(), form],
        None,
        None,
        limit
    )
    .is_err());
    assert!(CompanyFilingRequest::new(
        vec![company.clone()],
        vec![],
        Some(IsoDate::new("2026-01-01").unwrap()),
        None,
        limit
    )
    .is_err());
    assert!(CompanyFilingRequest::new(
        vec![company],
        vec![],
        None,
        Some(IsoDate::new("2026-12-31").unwrap()),
        limit
    )
    .is_err());
}
