use magic_market_core::{
    AssetClass, Exchange, FinancialLine, FinancialStatement, FiniteNumber, InstrumentId, IsoDate,
    NonEmptyText, ProviderId, Quantity, SecurityProfile, SourceEvidence, SourcedRecord,
    StatementKind,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity).unwrap()
}

fn evidence() -> SourceEvidence {
    SourceEvidence::new(ProviderId::Sina, "observed", "batch").unwrap()
}

#[test]
fn statements_retain_stable_key_source_label_and_absence() {
    let statement = FinancialStatement {
        instrument: instrument(),
        kind: StatementKind::Income,
        report_period: IsoDate::new("2026-06-30").unwrap(),
        announced_on: None,
        currency: Some(NonEmptyText::new("CNY").unwrap()),
        lines: vec![
            FinancialLine {
                key: NonEmptyText::new("operating_revenue").unwrap(),
                source_label: NonEmptyText::new("营业收入").unwrap(),
                value: Some(FiniteNumber::new(100.0).unwrap()),
                unit: Some(NonEmptyText::new("元").unwrap()),
            },
            FinancialLine {
                key: NonEmptyText::new("net_profit").unwrap(),
                source_label: NonEmptyText::new("净利润").unwrap(),
                value: None,
                unit: Some(NonEmptyText::new("元").unwrap()),
            },
        ],
        evidence: evidence(),
    };

    assert!(statement.lines[1].value.is_none());
    assert_eq!(statement.provider_id(), ProviderId::Sina);
    assert_eq!(statement.evidence_batch_id(), "batch");
    assert_eq!(
        serde_json::from_str::<FinancialStatement>(&serde_json::to_string(&statement).unwrap())
            .unwrap(),
        statement
    );
}

#[test]
fn company_profile_is_typed_without_guessing_facts() {
    let profile = SecurityProfile {
        instrument: instrument(),
        name: NonEmptyText::new("华电辽能").unwrap(),
        industry: None,
        listed_on: None,
        total_shares: Some(Quantity::new(1_000.0).unwrap()),
        floating_shares: None,
        facts: vec![],
        evidence: evidence(),
    };
    assert!(profile.industry.is_none());
    assert_eq!(profile.total_shares.unwrap().get(), 1_000.0);
    assert_eq!(profile.provider_id(), ProviderId::Sina);
    assert_eq!(profile.evidence_batch_id(), "batch");
}
