use magic_market_core::{
    HttpsUrl, NonEmptyText, ProviderId, ResearchDocument, SourceEvidence, SourcedRecord,
};

fn document(body: &[u8]) -> Result<ResearchDocument, magic_market_core::CoreError> {
    ResearchDocument::new(
        NonEmptyText::new("AP202607231827290069").unwrap(),
        HttpsUrl::new("https://pdf.dfcfw.com/pdf/H3_AP202607231827290069_1.pdf").unwrap(),
        body.to_vec(),
        SourceEvidence::new(ProviderId::Eastmoney, "observed", "batch").unwrap(),
    )
}

#[test]
fn research_document_accepts_terminal_eof_with_pdf_trailing_whitespace() {
    let body = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\nstartxref\n9\n%%EOF\0\t\n\x0c\r ";
    assert!(document(body).is_ok());
}

#[test]
fn research_document_rejects_a_truncated_body_without_eof() {
    let body = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\nstartxref\n9\n";
    assert!(document(body).is_err());
}

#[test]
fn research_document_rejects_non_whitespace_after_eof() {
    let body = b"%PDF-1.7\nstartxref\n9\n%%EOF\ntruncated";
    assert!(document(body).is_err());
}

#[test]
fn research_document_rejects_terminal_eof_without_startxref() {
    let body = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n%%EOF\n";
    assert!(document(body).is_err());
}

#[test]
fn research_document_rejects_invalid_header_empty_and_oversized_bodies() {
    assert!(document(b"not-a-pdf\nstartxref\n9\n%%EOF").is_err());
    assert!(document(b"").is_err());
    assert!(document(b"   \n\t").is_err());

    let mut oversized = vec![b'x'; 32 * 1024 * 1024 + 1];
    oversized[..5].copy_from_slice(b"%PDF-");
    assert!(document(&oversized).is_err());
}

#[test]
fn research_document_checked_deserialization_preserves_valid_pdf() {
    let original = document(b"%PDF-1.7\nstartxref\n9\n%%EOF\n").unwrap();
    let restored: ResearchDocument =
        serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();
    assert_eq!(restored, original);
    assert_eq!(restored.provider_id(), ProviderId::Eastmoney);
    assert_eq!(restored.evidence_batch_id(), "batch");

    let mut invalid = serde_json::to_value(original).unwrap();
    invalid["body"] = serde_json::json!([110, 111, 116, 45, 97, 45, 112, 100, 102]);
    assert!(serde_json::from_value::<ResearchDocument>(invalid).is_err());

    let mut invalid_content_type =
        serde_json::to_value(document(b"%PDF-1.7\nstartxref\n9\n%%EOF\n").unwrap()).unwrap();
    invalid_content_type["content_type"] = serde_json::json!("application/octet-stream");
    assert!(serde_json::from_value::<ResearchDocument>(invalid_content_type).is_err());
}
