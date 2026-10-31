use magic_market_core::{DataBatch, Provenance};
#[test]
fn strict_batch_preserves_metadata() {
    let p = Provenance::new("fixture", "now")
        .unwrap()
        .with_source_at("source")
        .unwrap();
    let b = DataBatch::strict(vec![1, 2], p.clone());
    assert_eq!(b.records(), &[1, 2]);
    assert_eq!(b.provenance(), &p);
    assert!(b.quality().is_complete());
}

#[test]
fn best_effort_preserves_quality_issues() {
    let batch = DataBatch::best_effort(
        vec![1],
        Provenance::new("fixture", "now").unwrap(),
        vec!["missing page".into()],
    )
    .unwrap();
    assert!(!batch.quality().is_complete());
    assert_eq!(batch.quality().issues(), ["missing page"]);
}

#[test]
fn provenance_and_quality_reject_empty_evidence() {
    assert!(Provenance::new(" ", "now").is_err());
    assert!(Provenance::new("fixture", " ").is_err());
    assert!(DataBatch::<u8>::best_effort(
        vec![],
        Provenance::new("fixture", "now").unwrap(),
        vec![" ".into()],
    )
    .is_err());
    assert!(DataBatch::<u8>::best_effort(
        vec![],
        Provenance::new("fixture", "now").unwrap(),
        vec!["bad\nissue".into()],
    )
    .is_err());
    assert!(Provenance::new("bad\nsource", "now").is_err());
}
