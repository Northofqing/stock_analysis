use super::*;
use codec_v1::OWNED_HITS;
const DRAFT_GOLDEN: &str = r#"{"schema":"retention-package-draft-v1","schema_version":1,"trust":"Unverified","owner_domain":"Data","owner_schema_claim":"example-unverified","logical_slot_claim":"example-slot","business_day_claim":"1970-01-01","window_start_claim":{"unix_seconds":0,"nanosecond":0},"window_end_exclusive_claim":{"unix_seconds":1,"nanosecond":0},"claimed_record_count":0,"source_chain_before_claim":null,"source_chain_after_claim":null,"artifact_sha256_claim":null,"activation_id_claim":null,"body_encoding":"hex","body_length":0,"body_sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","body_hex":""}"#;
const RECEIPT_GOLDEN: &str = r#"{"schema":"retention-stored-version-receipt-v1","schema_version":1,"trust":"Unverified","package_id":"retention-package-draft-v1:cce637d2a0302c5755ee3b51f2233228ad9677ad007b7cdc18950d597f285068","storage_authority_claim":"example-unconfigured","container_claim":"example-container","object_key_claim":"example-key","version_id_claim":"example-version","retention_mode_claim":"Compliance","clock_evidence":"Unverified","confirmation_upper_bound_claim":{"unix_seconds":0,"nanosecond":0},"retain_until_claim":{"unix_seconds":158112000,"nanosecond":0},"head_content_length_claim":610,"get_content_length_claim":610,"get_sha256_claim":"436d7459e7e881890ee87a445dc163818de11562d531991abe4b701ed44901f3","readback_complete_claim":true,"request_id_claims":[]}"#;
const ROOT_GOLDEN: &str = r#"{"schema":"retention-incomplete-daily-root-v1","schema_version":1,"trust":"Unverified","coverage_state":"Incomplete","signature_state":"Unsigned","authority_gates":{"owner_seal":"NotObserved","remote_retention":"NotObserved","signer":"NotConfigured","restore":"NotObserved"},"business_day_claim":"1970-01-01","revision":1,"previous_day_root_id_claim":null,"previous_revision_root_id_claim":null,"coverage":[{"owner_domain":"Data","state":"UnverifiedMaterialProvided","package_count":1},{"owner_domain":"InvestmentDecision","state":"NoMaterialProvided","package_count":0},{"owner_domain":"PaperLedger","state":"NoMaterialProvided","package_count":0},{"owner_domain":"Attribution","state":"NoMaterialProvided","package_count":0}],"entries":[{"owner_domain":"Data","logical_slot_claim":"example-slot","package_id":"retention-package-draft-v1:cce637d2a0302c5755ee3b51f2233228ad9677ad007b7cdc18950d597f285068","package_canonical_sha256":"436d7459e7e881890ee87a445dc163818de11562d531991abe4b701ed44901f3","package_canonical_length":610,"receipt_id":"retention-stored-receipt-v1:87130be0ec75beba8f56dbd599ce3d9341e43c1dc9f9ed0c7404b0f89bfb7a55","receipt_claim_consistency":"ConsistentClaims"}]}"#;

fn claims<'a>(owner: OwnerDomain, slot: &'a str) -> DraftClaimsRef<'a> {
    DraftClaimsRef {
        owner_domain: owner,
        owner_schema_claim: "example-unverified",
        logical_slot_claim: slot,
        business_day_claim: "1970-01-01",
        window_start_claim: UtcInstantClaim {
            unix_seconds: 0,
            nanosecond: 0,
        },
        window_end_exclusive_claim: UtcInstantClaim {
            unix_seconds: 1,
            nanosecond: 0,
        },
        claimed_record_count: Some(0),
        source_chain_before_claim: None,
        source_chain_after_claim: None,
        artifact_sha256_claim: None,
        activation_id_claim: None,
    }
}
fn root_claims() -> DailyRootClaimsRef<'static> {
    DailyRootClaimsRef {
        business_day_claim: "1970-01-01",
        revision: 1,
        previous_day_root_id_claim: None,
        previous_revision_root_id_claim: None,
    }
}
fn preowned_draft(b: &str, expected: ValueError) {
    OWNED_HITS.with(|h| h.set(0));
    assert_eq!(parse_draft(b.as_bytes()).err(), Some(expected));
    OWNED_HITS.with(|h| assert_eq!(h.get(), 0));
}
#[test]
fn retention_draft_literal_golden_and_build_agree() {
    let d = parse_draft(DRAFT_GOLDEN.as_bytes()).unwrap();
    assert_eq!(d.id(),"retention-package-draft-v1:cce637d2a0302c5755ee3b51f2233228ad9677ad007b7cdc18950d597f285068");
    let built = draft_from_claims(claims(OwnerDomain::Data, "example-slot"), b"").unwrap();
    assert_eq!(built.as_canonical_bytes(), DRAFT_GOLDEN.as_bytes());
    assert_eq!(built.id(), d.id());
    assert_eq!(d.trust(), TrustState::Unverified);
}
#[test]
fn retention_receipt_literal_golden_is_only_consistent_claims() {
    let d = parse_draft(DRAFT_GOLDEN.as_bytes()).unwrap();
    let r = parse_stored_receipt(RECEIPT_GOLDEN.as_bytes()).unwrap();
    assert_eq!(r.id(),"retention-stored-receipt-v1:87130be0ec75beba8f56dbd599ce3d9341e43c1dc9f9ed0c7404b0f89bfb7a55");
    assert_eq!(r.as_canonical_bytes(), RECEIPT_GOLDEN.as_bytes());
    let c = check_stored_receipt(&d, &r).unwrap();
    assert_eq!(c.consistency(), ClaimConsistency::ConsistentClaims);
    assert_eq!(c.arithmetic(), RetentionArithmetic::SufficientClaim);
    assert!(c.issues().is_empty());
    assert_eq!(c.trust(), TrustState::Unverified);
    assert_eq!(r.trust(), TrustState::Unverified);
}
#[test]
fn retention_root_literal_golden_and_build_agree() {
    let d = parse_draft(DRAFT_GOLDEN.as_bytes()).unwrap();
    let r = parse_stored_receipt(RECEIPT_GOLDEN.as_bytes()).unwrap();
    let root = build_daily_root(
        root_claims(),
        &[DraftAndReceiptRef {
            draft: &d,
            receipt: Some(&r),
        }],
    )
    .unwrap();
    assert_eq!(root.as_canonical_bytes(), ROOT_GOLDEN.as_bytes());
    assert_eq!(root.id(),"retention-incomplete-root-v1:3a502fb1b4630e8653700794cbcd776255316cd6a27743cb8dac8d9a4934eab6");
    assert_eq!(
        parse_daily_root(ROOT_GOLDEN.as_bytes()).unwrap().id(),
        root.id()
    );
    assert_eq!(root.trust(), TrustState::Unverified);
    assert_eq!(root.coverage_state(), CoverageState::Incomplete);
    assert_eq!(root.signature_state(), SignatureState::Unsigned);
}
#[test]
fn retention_closed_shape_rejects_before_owned() {
    for (b, e) in [
        (
            DRAFT_GOLDEN.replacen("\"schema\":", "\"unknown\":", 1),
            ValueError::UnknownField,
        ),
        (
            DRAFT_GOLDEN.replacen("\"schema_version\":1", "\"schema\":1", 1),
            ValueError::DuplicateField,
        ),
        (
            DRAFT_GOLDEN.replacen("\"schema_version\":1,", "", 1),
            ValueError::NonCanonical,
        ),
        (
            DRAFT_GOLDEN.replacen(
                "\"claimed_record_count\":0",
                "\"claimed_record_count\":[[0]]",
                1,
            ),
            ValueError::WrongType,
        ),
        (
            DRAFT_GOLDEN.replacen("\"schema\":", r#""sch\\u0065ma":"#, 1),
            ValueError::NonCanonical,
        ),
    ] {
        preowned_draft(&b, e);
    }
}
#[test]
fn retention_same_name_hex_and_numeric_bombs_have_no_exemption() {
    let bomb = format!("[{}]", vec!["0"; 10000].join(","));
    preowned_draft(
        &DRAFT_GOLDEN.replace(
            "\"claimed_record_count\":0",
            &format!("\"claimed_record_count\":{bomb}"),
        ),
        ValueError::WrongType,
    );
    let wrong = DRAFT_GOLDEN.replace(
        "\"window_start_claim\":{",
        &format!(
            "\"window_start_claim\":{{\"body_hex\":\"{}\",",
            "a".repeat(20000)
        ),
    );
    preowned_draft(&wrong, ValueError::UnknownField);
    let escaped = DRAFT_GOLDEN.replace("example-unverified", &r#"\\\""#.repeat(129));
    preowned_draft(&escaped, ValueError::InvalidScalar);
}
#[test]
fn retention_body_hash_length_and_maximum_boundary() {
    let raw = vec![0x5a; MIB];
    let d = draft_from_claims(claims(OwnerDomain::Data, "max"), &raw).unwrap();
    assert_eq!(parse_draft(d.as_canonical_bytes()).unwrap().id(), d.id());
    assert_eq!(
        draft_from_claims(claims(OwnerDomain::Data, "max"), &vec![0; MIB + 1]).err(),
        Some(ValueError::InputLimit)
    );
    assert_eq!(
        parse_draft(
            DRAFT_GOLDEN
                .replace("\"body_length\":0", "\"body_length\":1")
                .as_bytes()
        )
        .err(),
        Some(ValueError::BodyMismatch)
    );
    assert_eq!(
        parse_draft(DRAFT_GOLDEN.replace("e3b0c442", "03b0c442").as_bytes()).err(),
        Some(ValueError::BodyMismatch)
    );
}
#[test]
fn retention_escaped_unicode_uses_exact_owned_buffer() {
    let slot = "quote\\\"\\\\-汉-😀";
    let d = draft_from_claims(claims(OwnerDomain::Data, slot), b"x").unwrap();
    let p = parse_draft(d.as_canonical_bytes()).unwrap();
    assert_eq!(p.slot, slot);
    assert_eq!(p.id(), d.id());
    preowned_draft(
        &DRAFT_GOLDEN.replace("example-slot", r#"\uD800x"#),
        ValueError::MalformedJson,
    );
    assert_eq!(
        parse_draft(
            DRAFT_GOLDEN
                .replace("example-slot", r#"example-\u0073lot"#)
                .as_bytes()
        )
        .err(),
        Some(ValueError::NonCanonical)
    );
}
#[test]
fn retention_material_claims_change_only_draft_domain_id() {
    let a = draft_from_claims(claims(OwnerDomain::Data, "a"), b"x").unwrap();
    let b = draft_from_claims(claims(OwnerDomain::Data, "b"), b"x").unwrap();
    let c = draft_from_claims(claims(OwnerDomain::Data, "a"), b"y").unwrap();
    assert_ne!(a.id(), b.id());
    assert_ne!(a.id(), c.id());
    for d in [a, b, c] {
        assert!(d.id().starts_with(DP));
        assert_eq!(d.trust(), TrustState::Unverified);
    }
}
#[test]
fn retention_claim_issues_cover_hash_length_version_mode_and_readback() {
    let d = parse_draft(DRAFT_GOLDEN.as_bytes()).unwrap();
    for (from, to, issue) in [
        (
            "\"version_id_claim\":\"example-version\"",
            "\"version_id_claim\":null",
            ReceiptIssue::VersionMissing,
        ),
        (
            "\"Compliance\"",
            "\"Governance\"",
            ReceiptIssue::RetentionModeUnsupportedClaim,
        ),
        (
            "\"head_content_length_claim\":610",
            "\"head_content_length_claim\":609",
            ReceiptIssue::HeadLengthMismatch,
        ),
        (
            "\"get_content_length_claim\":610",
            "\"get_content_length_claim\":611",
            ReceiptIssue::GetLengthMismatch,
        ),
        ("436d7459", "036d7459", ReceiptIssue::GetHashMismatch),
        (
            "\"readback_complete_claim\":true",
            "\"readback_complete_claim\":false",
            ReceiptIssue::ReadbackIncompleteClaim,
        ),
    ] {
        let r = parse_stored_receipt(RECEIPT_GOLDEN.replace(from, to).as_bytes()).unwrap();
        let c = check_stored_receipt(&d, &r).unwrap();
        assert!(c.issues().contains(&issue));
        assert_eq!(c.consistency(), ClaimConsistency::InconsistentClaims);
        assert_eq!(c.trust(), TrustState::Unverified);
    }
}
#[test]
fn retention_1830_days_is_claim_arithmetic_with_nanoseconds_and_overflow() {
    let d = parse_draft(DRAFT_GOLDEN.as_bytes()).unwrap();
    let r = parse_stored_receipt(
        RECEIPT_GOLDEN
            .replace(
                "\"unix_seconds\":0,\"nanosecond\":0",
                "\"unix_seconds\":0,\"nanosecond\":1",
            )
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        check_stored_receipt(&d, &r).unwrap().arithmetic(),
        RetentionArithmetic::ShortClaim
    );
    let r = parse_stored_receipt(
        RECEIPT_GOLDEN
            .replace(
                "\"unix_seconds\":0,\"nanosecond\":0",
                "\"unix_seconds\":253402300799,\"nanosecond\":0",
            )
            .as_bytes(),
    )
    .unwrap();
    assert!(check_stored_receipt(&d, &r)
        .unwrap()
        .issues()
        .contains(&ReceiptIssue::TimeOverflow));
    let leap = UtcInstantClaim {
        unix_seconds: 1709164800,
        nanosecond: 123,
    };
    assert_eq!(
        leap.required().unwrap().unix_seconds - leap.unix_seconds,
        1830 * 86400
    );
    assert_eq!(leap.required().unwrap().nanosecond, 123);
    let r = parse_stored_receipt(
        RECEIPT_GOLDEN
            .replace(
                "\"confirmation_upper_bound_claim\":{\"unix_seconds\":0,\"nanosecond\":0}",
                "\"confirmation_upper_bound_claim\":null",
            )
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        check_stored_receipt(&d, &r).unwrap().arithmetic(),
        RetentionArithmetic::NotComputable
    );
}
#[test]
fn retention_all_four_materials_still_incomplete_unsigned_and_sort_stable() {
    let drafts: Vec<_> = DOMAINS
        .into_iter()
        .map(|o| draft_from_claims(claims(o, "slot"), b"x").unwrap())
        .collect();
    let receipts:Vec<_>=drafts.iter().map(|d|{
        let raw=RECEIPT_GOLDEN.replace("retention-package-draft-v1:cce637d2a0302c5755ee3b51f2233228ad9677ad007b7cdc18950d597f285068",d.id())
            .replace("436d7459e7e881890ee87a445dc163818de11562d531991abe4b701ed44901f3",&d.sha256)
            .replace(":610",&format!(":{}",d.canonical.len()));
        let r=parse_stored_receipt(raw.as_bytes()).unwrap();
        assert_eq!(check_stored_receipt(d,&r).unwrap().consistency(),ClaimConsistency::ConsistentClaims);r
    }).collect();
    let mut refs: Vec<_> = drafts
        .iter()
        .zip(&receipts)
        .map(|(d, r)| DraftAndReceiptRef {
            draft: d,
            receipt: Some(r),
        })
        .collect();
    let a = build_daily_root(root_claims(), &refs).unwrap();
    refs.reverse();
    let b = build_daily_root(root_claims(), &refs).unwrap();
    assert_eq!(a.as_canonical_bytes(), b.as_canonical_bytes());
    assert_eq!(a.coverage_state(), CoverageState::Incomplete);
    assert_eq!(a.signature_state(), SignatureState::Unsigned);
    let text = std::str::from_utf8(a.as_canonical_bytes()).unwrap();
    assert_eq!(text.matches("UnverifiedMaterialProvided").count(), 4);
    assert!(text.contains("\"owner_seal\":\"NotObserved\",\"remote_retention\":\"NotObserved\",\"signer\":\"NotConfigured\",\"restore\":\"NotObserved\""));
    let empty = build_daily_root(root_claims(), &[]).unwrap();
    assert_eq!(
        std::str::from_utf8(empty.as_canonical_bytes())
            .unwrap()
            .matches("NoMaterialProvided")
            .count(),
        4
    );
}
#[test]
fn retention_root_rejects_duplicate_slot_day_and_coverage_lies() {
    let d = parse_draft(DRAFT_GOLDEN.as_bytes()).unwrap();
    assert_eq!(
        build_daily_root(
            root_claims(),
            &[
                DraftAndReceiptRef {
                    draft: &d,
                    receipt: None
                },
                DraftAndReceiptRef {
                    draft: &d,
                    receipt: None
                }
            ]
        )
        .err(),
        Some(ValueError::LogicalSlotConflict)
    );
    let mut c = root_claims();
    c.business_day_claim = "1970-01-02";
    assert_eq!(
        build_daily_root(
            c,
            &[DraftAndReceiptRef {
                draft: &d,
                receipt: None
            }]
        )
        .err(),
        Some(ValueError::RootBindingMismatch)
    );
    assert_eq!(
        parse_daily_root(
            ROOT_GOLDEN
                .replacen("\"package_count\":1", "\"package_count\":0", 1)
                .as_bytes()
        )
        .err(),
        Some(ValueError::RootBindingMismatch)
    );
    assert!(parse_daily_root(
        ROOT_GOLDEN
            .replace("\"Incomplete\"", "\"Complete\"")
            .as_bytes()
    )
    .is_err());
}
#[test]
fn retention_root_128_boundary_and_129_before_owned() {
    let drafts: Vec<_> = (0..128)
        .map(|i| {
            draft_from_claims(claims(OwnerDomain::Data, &format!("slot-{i:03}")), b"").unwrap()
        })
        .collect();
    let mut refs: Vec<_> = drafts
        .iter()
        .map(|d| DraftAndReceiptRef {
            draft: d,
            receipt: None,
        })
        .collect();
    let r = build_daily_root(root_claims(), &refs).unwrap();
    assert_eq!(
        parse_daily_root(r.as_canonical_bytes()).unwrap().id(),
        r.id()
    );
    refs.push(DraftAndReceiptRef {
        draft: &drafts[0],
        receipt: None,
    });
    assert_eq!(
        build_daily_root(root_claims(), &refs).err(),
        Some(ValueError::InputLimit)
    );
}
#[test]
fn retention_shared_work_limits_are_cumulative_without_refund() {
    let mut w = Work::new();
    w.own(8 * MIB).unwrap();
    assert_eq!(w.own(1), Err(ValueError::AllocationLimit));
    assert_eq!(w.own(0), Err(ValueError::AllocationLimit));
    let mut w = Work::new();
    w.scan(32 * MIB).unwrap();
    assert_eq!(w.scan(1), Err(ValueError::InputLimit));
    let mut w = Work::new();
    for _ in 0..8192 {
        w.node().unwrap();
    }
    assert_eq!(w.node(), Err(ValueError::NodeLimit));
    let mut w = Work::new();
    w.own(8 * MIB - 1).unwrap();
    OWNED_HITS.with(|h| h.set(0));
    assert_eq!(
        codec_v1::decode::<DraftWire>(DRAFT_GOLDEN.as_bytes(), Shape::Draft, DRAFT_LIMIT, &mut w)
            .err(),
        Some(ValueError::AllocationLimit)
    );
    OWNED_HITS.with(|h| assert_eq!(h.get(), 0));
}
#[test]
fn retention_root_receipt_checks_share_the_outer_scan_budget() {
    let d = draft_from_claims(claims(OwnerDomain::Data, "large"), &vec![0; MIB]).unwrap();
    let r = parse_stored_receipt(RECEIPT_GOLDEN.as_bytes()).unwrap();
    let mut w = Work::new();
    for _ in 0..7 {
        check_with(&d, &r, &mut w).unwrap();
    }
    assert_eq!(
        check_with(&d, &r, &mut w).err(),
        Some(ValueError::InputLimit)
    );
}

#[test]
fn retention_outer_root_reuses_budget_across_large_receipts() {
    let raw = vec![0; MIB];
    let drafts: Vec<_> = (0..8)
        .map(|i| draft_from_claims(claims(OwnerDomain::Data, &format!("large-{i}")), &raw).unwrap())
        .collect();
    let r = parse_stored_receipt(RECEIPT_GOLDEN.as_bytes()).unwrap();
    let refs: Vec<_> = drafts
        .iter()
        .map(|d| DraftAndReceiptRef {
            draft: d,
            receipt: Some(&r),
        })
        .collect();
    assert_eq!(
        build_daily_root(root_claims(), &refs).err(),
        Some(ValueError::InputLimit)
    );
}
#[test]
fn retention_receipt_and_root_collection_attacks_refuse_before_owned() {
    let mut r = RECEIPT_GOLDEN.replace("[]", r#"["a","b","c","d","e"]"#);
    OWNED_HITS.with(|h| h.set(0));
    assert_eq!(
        parse_stored_receipt(r.as_bytes()).err(),
        Some(ValueError::InputLimit)
    );
    OWNED_HITS.with(|h| assert_eq!(h.get(), 0));
    r = RECEIPT_GOLDEN.replace("[]", "[[[[0]]]]");
    OWNED_HITS.with(|h| h.set(0));
    assert_eq!(
        parse_stored_receipt(r.as_bytes()).err(),
        Some(ValueError::WrongType)
    );
    OWNED_HITS.with(|h| assert_eq!(h.get(), 0));
    let root = ROOT_GOLDEN.replace(
        "\"authority_gates\":{",
        "\"authority_gates\":{\"body_hex\":\"a\",",
    );
    OWNED_HITS.with(|h| h.set(0));
    assert_eq!(
        parse_daily_root(root.as_bytes()).err(),
        Some(ValueError::UnknownField)
    );
    OWNED_HITS.with(|h| assert_eq!(h.get(), 0));
}
