use magic_market_core::{
    AssetClass, CorporateAction, CorporateActionCategory, CorporateActionRequest,
    CorporateActionResponse, CorporateActionStatus, CorporateActionTerms, DataBatch, Exchange,
    FiniteNumber, InstrumentId, IsoDate, Price, Provenance, ProviderId, Ratio, RatioUnit,
    SecurityMetadata, SourceEvidence, SourcedRecord, UnverifiedSourceUnit,
};

fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap()
}

fn evidence(batch_id: &str) -> SourceEvidence {
    SourceEvidence::new(ProviderId::Tdx, "2026-07-27T10:00:00+08:00", batch_id).unwrap()
}

fn admission_as_of() -> IsoDate {
    IsoDate::new("2026-07-27").unwrap()
}

#[test]
fn distribution_preserves_checked_per_share_terms_and_evidence() {
    let action = CorporateAction::new(
        instrument(),
        CorporateActionCategory::Distribution,
        IsoDate::new("2025-06-27").unwrap(),
        CorporateActionStatus::Implemented,
        CorporateActionTerms::distribution(
            Some(FiniteNumber::new(2.4).unwrap()),
            Some(FiniteNumber::new(0.1).unwrap()),
            Some(FiniteNumber::new(0.05).unwrap()),
            Some(Price::new(120.0).unwrap()),
        )
        .unwrap(),
        evidence("tdx-actions"),
    )
    .unwrap();

    assert_eq!(action.instrument(), &instrument());
    assert_eq!(action.category(), CorporateActionCategory::Distribution);
    assert_eq!(action.effective_on().as_str(), "2025-06-27");
    assert_eq!(action.status(), CorporateActionStatus::Implemented);
    assert!(action.record_on().is_none());
    assert!(action.ex_on().is_none());
    assert!(action.payable_on().is_none());
    assert_eq!(action.provider_id(), ProviderId::Tdx);
    assert_eq!(action.evidence_batch_id(), "tdx-actions");
    assert_eq!(
        action.evidence_observed_at(),
        Some("2026-07-27T10:00:00+08:00")
    );
    assert_eq!(action.evidence_source_at(), None);
}

#[test]
fn verified_capital_rescaling_terms_require_positive_non_identity_decimal_ratios() {
    for category in [
        CorporateActionCategory::CapitalRescaling,
        CorporateActionCategory::NonTradableReverseSplit,
    ] {
        assert!(CorporateActionTerms::capital_rescaling(
            category,
            Ratio::new(0.0, RatioUnit::Decimal).unwrap()
        )
        .is_err());
        assert!(CorporateActionTerms::capital_rescaling(
            category,
            Ratio::new(1.0, RatioUnit::Decimal).unwrap()
        )
        .is_err());
        assert!(CorporateActionTerms::capital_rescaling(
            category,
            Ratio::new(2.0, RatioUnit::Percent).unwrap()
        )
        .is_err());
        assert!(CorporateActionTerms::capital_rescaling(
            category,
            Ratio::new(2.0, RatioUnit::Decimal).unwrap()
        )
        .is_ok());
    }
    assert!(CorporateActionTerms::capital_rescaling(
        CorporateActionCategory::Distribution,
        Ratio::new(2.0, RatioUnit::Decimal).unwrap()
    )
    .is_err());
}

#[test]
fn provider_native_rescaling_terms_do_not_claim_a_verified_ratio_unit() {
    for category in [
        CorporateActionCategory::CapitalRescaling,
        CorporateActionCategory::NonTradableReverseSplit,
    ] {
        let terms = CorporateActionTerms::provider_native_ratio(
            category,
            FiniteNumber::new(2.0).unwrap(),
            UnverifiedSourceUnit::ProviderNative,
        )
        .unwrap();
        assert_eq!(terms.category(), category);
    }
    assert!(CorporateActionTerms::provider_native_ratio(
        CorporateActionCategory::Distribution,
        FiniteNumber::new(2.0).unwrap(),
        UnverifiedSourceUnit::ProviderNative,
    )
    .is_err());
    assert!(CorporateActionTerms::provider_native_ratio(
        CorporateActionCategory::CapitalRescaling,
        FiniteNumber::new(0.0).unwrap(),
        UnverifiedSourceUnit::ProviderNative,
    )
    .is_err());
}

#[test]
fn capital_structure_terms_cover_exact_protocol_categories_with_unverified_units() {
    let categories = [
        CorporateActionCategory::BonusRightsListing,
        CorporateActionCategory::NonTradableShareListing,
        CorporateActionCategory::UnknownCapitalChange,
        CorporateActionCategory::CapitalChange,
        CorporateActionCategory::AdditionalIssuance,
        CorporateActionCategory::ShareRepurchase,
        CorporateActionCategory::AdditionalIssuanceListing,
        CorporateActionCategory::TransferredAllotmentListing,
        CorporateActionCategory::ConvertibleBondListing,
    ];
    for category in categories {
        let terms = CorporateActionTerms::capital_structure(
            category,
            FiniteNumber::new(10.0).unwrap(),
            FiniteNumber::new(11.0).unwrap(),
            FiniteNumber::new(20.0).unwrap(),
            FiniteNumber::new(21.0).unwrap(),
            UnverifiedSourceUnit::ProviderNative,
        )
        .unwrap();
        assert_eq!(terms.category(), category);
        assert!(CorporateAction::new(
            instrument(),
            category,
            IsoDate::new("2025-06-27").unwrap(),
            CorporateActionStatus::Implemented,
            terms,
            evidence("tdx-capital"),
        )
        .is_ok());
    }

    assert!(CorporateActionTerms::capital_structure(
        CorporateActionCategory::Distribution,
        FiniteNumber::new(10.0).unwrap(),
        FiniteNumber::new(11.0).unwrap(),
        FiniteNumber::new(20.0).unwrap(),
        FiniteNumber::new(21.0).unwrap(),
        UnverifiedSourceUnit::ProviderNative,
    )
    .is_err());
    assert!(CorporateActionTerms::capital_structure(
        CorporateActionCategory::CapitalChange,
        FiniteNumber::new(-1.0).unwrap(),
        FiniteNumber::new(11.0).unwrap(),
        FiniteNumber::new(20.0).unwrap(),
        FiniteNumber::new(21.0).unwrap(),
        UnverifiedSourceUnit::ProviderNative,
    )
    .is_err());
}

#[test]
fn warrant_grant_terms_require_exact_category_and_positive_values() {
    for category in [
        CorporateActionCategory::SubscriptionWarrantGrant,
        CorporateActionCategory::PutWarrantGrant,
    ] {
        let terms = CorporateActionTerms::warrant_grant(
            category,
            Price::new(30.3).unwrap(),
            FiniteNumber::new(16.0).unwrap(),
            UnverifiedSourceUnit::ProviderNative,
        )
        .unwrap();
        assert_eq!(terms.category(), category);
    }
    assert!(CorporateActionTerms::warrant_grant(
        CorporateActionCategory::Distribution,
        Price::new(30.3).unwrap(),
        FiniteNumber::new(16.0).unwrap(),
        UnverifiedSourceUnit::ProviderNative,
    )
    .is_err());
    assert!(CorporateActionTerms::warrant_grant(
        CorporateActionCategory::PutWarrantGrant,
        Price::new(30.3).unwrap(),
        FiniteNumber::new(0.0).unwrap(),
        UnverifiedSourceUnit::ProviderNative,
    )
    .is_err());
}

#[test]
fn checked_term_deserialization_rejects_mismatched_categories_and_bad_quantities() {
    let capital = CorporateActionTerms::capital_structure(
        CorporateActionCategory::CapitalChange,
        FiniteNumber::new(10.0).unwrap(),
        FiniteNumber::new(11.0).unwrap(),
        FiniteNumber::new(20.0).unwrap(),
        FiniteNumber::new(21.0).unwrap(),
        UnverifiedSourceUnit::ProviderNative,
    )
    .unwrap();
    let mut invalid_capital = serde_json::to_value(capital).unwrap();
    invalid_capital["CapitalStructure"]["category"] = serde_json::json!("Distribution");
    assert!(serde_json::from_value::<CorporateActionTerms>(invalid_capital).is_err());

    let warrant = CorporateActionTerms::warrant_grant(
        CorporateActionCategory::PutWarrantGrant,
        Price::new(30.3).unwrap(),
        FiniteNumber::new(16.0).unwrap(),
        UnverifiedSourceUnit::ProviderNative,
    )
    .unwrap();
    let mut invalid_warrant = serde_json::to_value(warrant).unwrap();
    invalid_warrant["WarrantGrant"]["source_quantity"] = serde_json::json!(0.0);
    assert!(serde_json::from_value::<CorporateActionTerms>(invalid_warrant).is_err());
}

#[test]
fn action_rejects_category_or_terms_disagreement_but_preserves_source_status() {
    let split = CorporateActionTerms::capital_rescaling(
        CorporateActionCategory::CapitalRescaling,
        Ratio::new(2.0, RatioUnit::Decimal).unwrap(),
    )
    .unwrap();
    assert!(CorporateAction::new(
        instrument(),
        CorporateActionCategory::NonTradableReverseSplit,
        IsoDate::new("2025-06-27").unwrap(),
        CorporateActionStatus::Implemented,
        split,
        evidence("tdx-actions"),
    )
    .is_err());
    for status in [
        CorporateActionStatus::Proposed,
        CorporateActionStatus::Cancelled,
        CorporateActionStatus::Unknown,
    ] {
        let action = CorporateAction::new(
            instrument(),
            CorporateActionCategory::CapitalRescaling,
            IsoDate::new("2025-06-27").unwrap(),
            status,
            CorporateActionTerms::capital_rescaling(
                CorporateActionCategory::CapitalRescaling,
                Ratio::new(2.0, RatioUnit::Decimal).unwrap(),
            )
            .unwrap(),
            evidence("tdx-actions"),
        )
        .unwrap();
        assert_eq!(action.status(), status);
        assert!(!action.is_implemented());
    }

    assert!(CorporateActionTerms::distribution(
        Some(FiniteNumber::new(-0.01).unwrap()),
        None,
        None,
        None,
    )
    .is_err());
    assert!(CorporateActionTerms::distribution(None, None, None, None).is_err());
    assert!(CorporateActionTerms::distribution(
        Some(FiniteNumber::new(0.0).unwrap()),
        Some(FiniteNumber::new(0.0).unwrap()),
        Some(FiniteNumber::new(0.0).unwrap()),
        None,
    )
    .is_err());
    assert!(CorporateActionTerms::distribution(
        Some(FiniteNumber::new(1.0).unwrap()),
        None,
        Some(FiniteNumber::new(0.0).unwrap()),
        Some(Price::new(10.0).unwrap()),
    )
    .is_err());
}

#[test]
fn request_requires_an_ordered_two_sided_range_and_checked_serde() {
    let request = CorporateActionRequest::new(instrument())
        .with_range(
            IsoDate::new("2020-01-01").unwrap(),
            IsoDate::new("2025-12-31").unwrap(),
        )
        .unwrap();
    assert_eq!(request.instrument(), &instrument());
    assert_eq!(request.start().unwrap().as_str(), "2020-01-01");
    assert_eq!(request.end().unwrap().as_str(), "2025-12-31");
    assert!(CorporateActionRequest::new(instrument())
        .with_range(
            IsoDate::new("2025-12-31").unwrap(),
            IsoDate::new("2020-01-01").unwrap(),
        )
        .is_err());

    let json = serde_json::to_string(&request).unwrap();
    assert_eq!(
        serde_json::from_str::<CorporateActionRequest>(&json).unwrap(),
        request
    );
    let mut one_sided = serde_json::to_value(CorporateActionRequest::new(instrument())).unwrap();
    one_sided["start"] = serde_json::json!("2020-01-01");
    assert!(serde_json::from_value::<CorporateActionRequest>(one_sided).is_err());
}

#[test]
fn action_checked_serde_rejects_invalid_terms_and_security_metadata_is_sourced() {
    let action = CorporateAction::new(
        instrument(),
        CorporateActionCategory::CapitalRescaling,
        IsoDate::new("2025-06-27").unwrap(),
        CorporateActionStatus::Implemented,
        CorporateActionTerms::capital_rescaling(
            CorporateActionCategory::CapitalRescaling,
            Ratio::new(2.0, RatioUnit::Decimal).unwrap(),
        )
        .unwrap(),
        evidence("tdx-actions"),
    )
    .unwrap();
    let json = serde_json::to_string(&action).unwrap();
    assert_eq!(
        serde_json::from_str::<CorporateAction>(&json).unwrap(),
        action
    );

    let mut invalid = serde_json::to_value(&action).unwrap();
    invalid["category"] = serde_json::json!("NonTradableReverseSplit");
    assert!(serde_json::from_value::<CorporateAction>(invalid).is_err());
    let mut invalid_terms = serde_json::to_value(action.terms()).unwrap();
    invalid_terms["CapitalRescaling"]["ratio"]["value"] = serde_json::json!(1.0);
    assert!(serde_json::from_value::<CorporateActionTerms>(invalid_terms).is_err());
    let mut non_implemented = serde_json::to_value(action).unwrap();
    non_implemented["status"] = serde_json::json!("Proposed");
    let proposed = serde_json::from_value::<CorporateAction>(non_implemented).unwrap();
    assert_eq!(proposed.status(), CorporateActionStatus::Proposed);
    assert!(!proposed.is_implemented());

    fn assert_sourced<T: SourcedRecord>() {}
    assert_sourced::<SecurityMetadata>();
    assert_sourced::<CorporateAction>();
}

fn split_action(
    returned_instrument: InstrumentId,
    effective_on: &str,
    evidence: SourceEvidence,
) -> CorporateAction {
    CorporateAction::new(
        returned_instrument,
        CorporateActionCategory::CapitalRescaling,
        IsoDate::new(effective_on).unwrap(),
        CorporateActionStatus::Implemented,
        CorporateActionTerms::capital_rescaling(
            CorporateActionCategory::CapitalRescaling,
            Ratio::new(2.0, RatioUnit::Decimal).unwrap(),
        )
        .unwrap(),
        evidence,
    )
    .unwrap()
}

fn action_batch(record: CorporateAction, provenance: Provenance) -> DataBatch<CorporateAction> {
    DataBatch::strict(vec![record], provenance)
}

#[test]
fn corporate_action_response_checks_coverage_and_atomic_evidence() {
    let request = CorporateActionRequest::new(instrument())
        .with_range(
            IsoDate::new("2025-01-01").unwrap(),
            IsoDate::new("2025-12-31").unwrap(),
        )
        .unwrap();
    let observed_at = "2026-07-27T10:00:00+08:00";
    let source_at = "2026-07-27T09:59:59+08:00";
    let matched_evidence = SourceEvidence::new(ProviderId::Tdx, observed_at, "tdx-actions")
        .unwrap()
        .with_source_at(source_at)
        .unwrap();
    let provenance = Provenance::new("tdx", observed_at)
        .unwrap()
        .with_source_at(source_at)
        .unwrap()
        .with_batch_id("tdx-actions")
        .unwrap();
    let response = CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        matched_evidence.clone(),
        action_batch(
            split_action(instrument(), "2025-06-27", matched_evidence.clone()),
            provenance.clone(),
        ),
    )
    .unwrap();
    assert_eq!(response.coverage(), &request);
    assert_eq!(response.evidence(), &matched_evidence);
    assert_eq!(response.batch().records().len(), 1);
    assert_eq!(response.implemented_actions().count(), 1);
    assert_eq!(
        response.batch().records()[0].evidence_source_at(),
        Some(source_at)
    );

    let proposed_response = CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        matched_evidence.clone(),
        action_batch(
            CorporateAction::new(
                instrument(),
                CorporateActionCategory::CapitalRescaling,
                IsoDate::new("2025-06-27").unwrap(),
                CorporateActionStatus::Proposed,
                CorporateActionTerms::capital_rescaling(
                    CorporateActionCategory::CapitalRescaling,
                    Ratio::new(2.0, RatioUnit::Decimal).unwrap(),
                )
                .unwrap(),
                matched_evidence.clone(),
            )
            .unwrap(),
            provenance.clone(),
        ),
    )
    .unwrap();
    assert_eq!(
        proposed_response.batch().records()[0].status(),
        CorporateActionStatus::Proposed
    );
    assert_eq!(proposed_response.implemented_actions().count(), 0);

    let other = InstrumentId::new(Exchange::Shenzhen, "000001", AssetClass::Equity).unwrap();
    assert!(CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        matched_evidence.clone(),
        action_batch(
            split_action(other, "2025-06-27", matched_evidence.clone()),
            provenance.clone(),
        ),
    )
    .is_err());
    assert!(CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        matched_evidence.clone(),
        action_batch(
            split_action(instrument(), "2024-12-31", matched_evidence.clone()),
            provenance.clone(),
        ),
    )
    .is_err());
    assert!(CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        matched_evidence.clone(),
        action_batch(
            split_action(
                instrument(),
                "2025-06-27",
                SourceEvidence::new(ProviderId::Tdx, observed_at, "other-batch")
                    .unwrap()
                    .with_source_at(source_at)
                    .unwrap(),
            ),
            provenance.clone(),
        ),
    )
    .is_err());
    assert!(CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        matched_evidence.clone(),
        action_batch(
            split_action(
                instrument(),
                "2025-06-27",
                SourceEvidence::new(ProviderId::Tdx, "2026-07-27T10:00:01+08:00", "tdx-actions",)
                    .unwrap()
                    .with_source_at(source_at)
                    .unwrap(),
            ),
            provenance.clone(),
        ),
    )
    .is_err());
    assert!(CorporateActionResponse::new(
        request,
        admission_as_of(),
        matched_evidence,
        action_batch(
            split_action(
                instrument(),
                "2025-06-27",
                SourceEvidence::new(ProviderId::Tdx, observed_at, "tdx-actions")
                    .unwrap()
                    .with_source_at("2026-07-27T09:59:58+08:00")
                    .unwrap(),
            ),
            provenance,
        ),
    )
    .is_err());
}

#[test]
fn corporate_action_response_rejects_best_effort_duplicates_and_unordered_records() {
    let request = CorporateActionRequest::new(instrument());
    let observed_at = "2026-07-27T10:00:00+08:00";
    let batch_id = "tdx-response-shape";
    let matched_evidence = SourceEvidence::new(ProviderId::Tdx, observed_at, batch_id).unwrap();
    let provenance = Provenance::new("tdx", observed_at)
        .unwrap()
        .with_batch_id(batch_id)
        .unwrap();
    let first = split_action(instrument(), "2025-06-01", matched_evidence.clone());
    let second = split_action(instrument(), "2025-06-02", matched_evidence.clone());

    assert!(CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        matched_evidence.clone(),
        DataBatch::best_effort(
            vec![first.clone()],
            provenance.clone(),
            vec!["partial source packet".into()],
        )
        .unwrap(),
    )
    .is_err());
    assert!(CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        matched_evidence.clone(),
        DataBatch::strict(vec![first.clone(), first], provenance.clone(),),
    )
    .is_err());
    assert!(CorporateActionResponse::new(
        request,
        admission_as_of(),
        matched_evidence,
        DataBatch::strict(
            vec![
                second,
                split_action(
                    instrument(),
                    "2025-06-01",
                    SourceEvidence::new(ProviderId::Tdx, observed_at, batch_id).unwrap(),
                )
            ],
            provenance
        ),
    )
    .is_err());
}

#[test]
fn corporate_action_response_rejects_future_records_and_future_coverage() {
    let observed_at = "2026-07-27T10:00:00+08:00";
    let batch_id = "tdx-future";
    let matched_evidence = SourceEvidence::new(ProviderId::Tdx, observed_at, batch_id).unwrap();
    let provenance = Provenance::new("tdx", observed_at)
        .unwrap()
        .with_batch_id(batch_id)
        .unwrap();
    assert!(CorporateActionResponse::new(
        CorporateActionRequest::new(instrument()),
        admission_as_of(),
        matched_evidence.clone(),
        DataBatch::strict(
            vec![split_action(
                instrument(),
                "2026-07-28",
                matched_evidence.clone(),
            )],
            provenance.clone(),
        ),
    )
    .is_err());
    assert!(CorporateActionResponse::new(
        CorporateActionRequest::new(instrument())
            .with_range(
                IsoDate::new("2026-07-27").unwrap(),
                IsoDate::new("2026-07-28").unwrap(),
            )
            .unwrap(),
        admission_as_of(),
        matched_evidence,
        DataBatch::strict(Vec::new(), provenance),
    )
    .is_err());
}

#[test]
fn corporate_action_response_preserves_empty_coverage_and_checks_deserialization() {
    let request = CorporateActionRequest::new(instrument())
        .with_range(
            IsoDate::new("1900-01-01").unwrap(),
            IsoDate::new("1900-12-31").unwrap(),
        )
        .unwrap();
    let response = CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        SourceEvidence::new(ProviderId::Tdx, "2026-07-27T10:00:00+08:00", "tdx-empty").unwrap(),
        DataBatch::strict(
            Vec::new(),
            Provenance::new("tdx", "2026-07-27T10:00:00+08:00")
                .unwrap()
                .with_batch_id("tdx-empty")
                .unwrap(),
        ),
    )
    .unwrap();
    assert_eq!(response.coverage(), &request);
    assert!(response.batch().records().is_empty());
    let (parts_coverage, parts_as_of, parts_evidence, parts_batch) = response.clone().into_parts();
    assert_eq!(parts_coverage, request);
    assert_eq!(parts_as_of, admission_as_of());
    assert_eq!(parts_evidence.provider(), ProviderId::Tdx);
    assert!(parts_batch.records().is_empty());
    let json = serde_json::to_string(&response).unwrap();
    assert_eq!(
        serde_json::from_str::<CorporateActionResponse>(&json).unwrap(),
        response
    );

    let mut invalid = serde_json::to_value(&response).unwrap();
    invalid["batch"]["records"] = serde_json::json!([split_action(
        instrument(),
        "2025-06-27",
        SourceEvidence::new(ProviderId::Tdx, "2026-07-27T10:00:00+08:00", "tdx-empty",).unwrap(),
    )]);
    assert!(serde_json::from_value::<CorporateActionResponse>(invalid).is_err());

    let invalid_time_evidence =
        SourceEvidence::new(ProviderId::Tdx, "not-a-time", "tdx-empty").unwrap();
    assert!(CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        invalid_time_evidence,
        DataBatch::strict(
            Vec::new(),
            Provenance::new("tdx", "not-a-time")
                .unwrap()
                .with_batch_id("tdx-empty")
                .unwrap(),
        ),
    )
    .is_err());

    let date_only_evidence =
        SourceEvidence::new(ProviderId::Tdx, "2026-07-27", "tdx-empty").unwrap();
    assert!(CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        date_only_evidence,
        DataBatch::strict(
            Vec::new(),
            Provenance::new("tdx", "2026-07-27")
                .unwrap()
                .with_batch_id("tdx-empty")
                .unwrap(),
        ),
    )
    .is_err());

    let future_source_evidence =
        SourceEvidence::new(ProviderId::Tdx, "2026-07-27T10:00:00+08:00", "tdx-empty")
            .unwrap()
            .with_source_at("2026-07-27T10:00:01+08:00")
            .unwrap();
    assert!(CorporateActionResponse::new(
        request,
        admission_as_of(),
        future_source_evidence,
        DataBatch::strict(
            Vec::new(),
            Provenance::new("tdx", "2026-07-27T10:00:00+08:00")
                .unwrap()
                .with_source_at("2026-07-27T10:00:01+08:00")
                .unwrap()
                .with_batch_id("tdx-empty")
                .unwrap(),
        ),
    )
    .is_err());
}

#[test]
fn every_corporate_action_term_variant_round_trips_and_validates_in_an_action() {
    let variants = [
        CorporateActionTerms::distribution(Some(FiniteNumber::new(1.0).unwrap()), None, None, None)
            .unwrap(),
        CorporateActionTerms::capital_rescaling(
            CorporateActionCategory::CapitalRescaling,
            Ratio::new(2.0, RatioUnit::Decimal).unwrap(),
        )
        .unwrap(),
        CorporateActionTerms::capital_rescaling(
            CorporateActionCategory::NonTradableReverseSplit,
            Ratio::new(0.5, RatioUnit::Decimal).unwrap(),
        )
        .unwrap(),
        CorporateActionTerms::provider_native_ratio(
            CorporateActionCategory::CapitalRescaling,
            FiniteNumber::new(2.0).unwrap(),
            UnverifiedSourceUnit::ProviderNative,
        )
        .unwrap(),
        CorporateActionTerms::capital_structure(
            CorporateActionCategory::CapitalChange,
            FiniteNumber::new(10.0).unwrap(),
            FiniteNumber::new(11.0).unwrap(),
            FiniteNumber::new(20.0).unwrap(),
            FiniteNumber::new(21.0).unwrap(),
            UnverifiedSourceUnit::ProviderNative,
        )
        .unwrap(),
        CorporateActionTerms::warrant_grant(
            CorporateActionCategory::PutWarrantGrant,
            Price::new(30.3).unwrap(),
            FiniteNumber::new(16.0).unwrap(),
            UnverifiedSourceUnit::ProviderNative,
        )
        .unwrap(),
    ];

    for terms in variants {
        let restored: CorporateActionTerms =
            serde_json::from_value(serde_json::to_value(&terms).unwrap()).unwrap();
        assert_eq!(restored, terms);
        let category = terms.category();
        let action = CorporateAction::new(
            instrument(),
            category,
            IsoDate::new("2025-06-27").unwrap(),
            CorporateActionStatus::Implemented,
            terms,
            evidence("term-variant"),
        )
        .unwrap();
        assert_eq!(action.terms().category(), category);
    }
}

#[test]
fn unbounded_request_and_response_consuming_accessors_round_trip() {
    let request = CorporateActionRequest::new(instrument());
    assert!(request.start().is_none());
    assert!(request.end().is_none());
    assert_eq!(
        serde_json::from_value::<CorporateActionRequest>(serde_json::to_value(&request).unwrap())
            .unwrap(),
        request
    );

    let observed_at = "2026-07-27T10:00:00+08:00";
    let response = CorporateActionResponse::new(
        request,
        admission_as_of(),
        SourceEvidence::new(ProviderId::Tdx, observed_at, "empty-accessors").unwrap(),
        DataBatch::strict(
            Vec::new(),
            Provenance::new("tdx", observed_at)
                .unwrap()
                .with_batch_id("empty-accessors")
                .unwrap(),
        ),
    )
    .unwrap();
    assert_eq!(response.admission_as_of(), &admission_as_of());
    assert_eq!(response.evidence().batch_id(), "empty-accessors");
    assert!(response.clone().into_batch().records().is_empty());
}

#[test]
fn response_rejects_missing_or_disagreeing_batch_provenance() {
    let request = CorporateActionRequest::new(instrument());
    let observed_at = "2026-07-27T10:00:00+08:00";
    let source_at = "2026-07-27T09:59:59+08:00";
    let response_evidence = SourceEvidence::new(ProviderId::Tdx, observed_at, "response")
        .unwrap()
        .with_source_at(source_at)
        .unwrap();

    assert!(CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        response_evidence.clone(),
        DataBatch::strict(
            Vec::new(),
            serde_json::from_value::<Provenance>(serde_json::json!({
                "source": "tdx",
                "source_at": source_at,
                "fetched_at": observed_at,
                "batch_id": null
            }))
            .unwrap(),
        ),
    )
    .is_err());
    assert!(CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        response_evidence.clone(),
        DataBatch::strict(
            Vec::new(),
            Provenance::new("tdx", observed_at)
                .unwrap()
                .with_source_at(source_at)
                .unwrap()
                .with_batch_id("other")
                .unwrap(),
        ),
    )
    .is_err());
    assert!(CorporateActionResponse::new(
        request.clone(),
        admission_as_of(),
        response_evidence.clone(),
        DataBatch::strict(
            Vec::new(),
            Provenance::new("tdx", "2026-07-27T10:00:01+08:00")
                .unwrap()
                .with_source_at(source_at)
                .unwrap()
                .with_batch_id("response")
                .unwrap(),
        ),
    )
    .is_err());
    assert!(CorporateActionResponse::new(
        request,
        admission_as_of(),
        response_evidence,
        DataBatch::strict(
            Vec::new(),
            Provenance::new("tdx", observed_at)
                .unwrap()
                .with_source_at("2026-07-27T09:59:58+08:00")
                .unwrap()
                .with_batch_id("response")
                .unwrap(),
        ),
    )
    .is_err());
}
