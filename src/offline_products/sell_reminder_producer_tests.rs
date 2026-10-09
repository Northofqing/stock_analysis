use super::*;
use crate::offline_products::shanghai_clock;

fn clock(value: &str) -> Clock {
    shanghai_clock(value).unwrap()
}
fn start() -> Clock {
    clock("2026-09-28T15:02:00+08:00")
}
fn fresh_pack(now: Clock) -> EvidencePack {
    let mut pack = super::super::tests::fixture(8., 60);
    let account = pack.account.as_mut().unwrap();
    account.captured_at = now;
    account.observed_at = now;
    account.source.known_at = now;
    for lot in &mut pack.lots {
        lot.source.known_at = now;
        lot.sell_fees.as_mut().unwrap().source.known_at = now;
    }
    for security in &mut pack.securities {
        for source in [
            &mut security.close_source,
            &mut security.status_source,
            &mut security.quantity.source,
            &mut security.bars_source,
        ] {
            source.known_at = now;
        }
    }
    pack
}
// This seam exists only in tests; no public DTO/file can call it in production.
fn qualified(pack: &EvidencePack, started: Clock, completed: Clock) -> Production {
    assemble(
        super::super::evaluate(pack, completed, Some(&QualifiedSource)),
        started,
        completed,
        Some(&QualifiedSource),
    )
    .unwrap()
}
#[test]
fn raw_observations_cannot_mint_producer_dispatch_even_when_all_fields_look_complete() {
    let completed = start() + Duration::minutes(1);
    let pack = fresh_pack(completed);
    let production = produce_observed(&pack, start(), completed).unwrap();
    assert_eq!(production.receipt().state, ProductionState::NotReady);
    assert!(production
        .receipt()
        .missing
        .iter()
        .any(|gap| gap.contains("ContractNotDelivered")));
    let exported = serde_json::to_value(production.receipt()).unwrap();
    assert_eq!(exported["dispatch_candidate_count"], 0);
    assert_eq!(exported["preview"]["rows"][0]["suggested_shares"], 0);
    assert!(exported["preview"]["rows"][0]["eligible_sellable"].is_null());
    assert!(production.into_candidate().is_none());
    let mut forged = serde_json::to_value(pack).unwrap();
    forged["source_qualified"] = true.into();
    assert!(serde_json::from_value::<EvidencePack>(forged).is_err());
}
#[test]
fn preparation_budget_and_source_rejection_are_independent() {
    for (started, completed, expected) in [
        (
            start() - Duration::milliseconds(1),
            start(),
            ProductionState::OutsidePreparationWindow,
        ),
        (
            start(),
            start() + Duration::milliseconds(120_000),
            ProductionState::Ready,
        ),
        (
            start(),
            start() + Duration::milliseconds(120_001),
            ProductionState::StartupBudgetExceeded,
        ),
        (
            start() + Duration::seconds(30),
            start() + Duration::milliseconds(120_001),
            ProductionState::StartupBudgetExceeded,
        ),
        (
            start() + Duration::minutes(2),
            start() + Duration::minutes(2),
            ProductionState::OutsidePreparationWindow,
        ),
        (
            start(),
            at(start().date_naive(), 15, 30),
            ProductionState::Expired,
        ),
    ] {
        let production = qualified(&fresh_pack(completed), started, completed);
        assert_eq!(
            production.receipt().state,
            expected,
            "{started} -> {completed}"
        );
    }
    assert!(produce_observed(
        &fresh_pack(start()),
        start(),
        start() - Duration::milliseconds(1)
    )
    .is_err());
    let mut pack = fresh_pack(start() + Duration::minutes(1));
    pack.lots[0].reserved = None;
    assert_eq!(
        qualified(&pack, start(), start() + Duration::minutes(1))
            .receipt()
            .state,
        ProductionState::NotReady
    );
    // A mixed inventory cannot silently deliver only its qualified-looking row.
    let mut second = pack.lots[0].clone();
    second.lot_id = Some("second-lot".into());
    second.reserved = Some(0);
    pack.lots.push(second);
    assert_eq!(
        qualified(&pack, start(), start() + Duration::minutes(1))
            .receipt()
            .state,
        ProductionState::NotReady
    );
}
#[test]
fn owner_consumption_rechecks_freshness_and_expiry_without_clock_resurrection() {
    let completed = start() + Duration::seconds(59);
    let mut candidate = qualified(&fresh_pack(completed), start(), completed)
        .into_candidate()
        .unwrap();
    assert!(matches!(
        candidate.prepare_dispatch(completed),
        Err(DispatchBlock::BeforeWindow)
    ));
    let ready = candidate
        .prepare_dispatch(start() + Duration::minutes(1))
        .unwrap();
    assert!(!ready.retry_authorized());
    assert_eq!(ready.expires_at(), at(start().date_naive(), 15, 30));
    assert_eq!(
        ready.validate_consumption_at(at(start().date_naive(), 15, 30)),
        Err(DispatchBlock::Expired)
    );
    assert_eq!(
        ready.validate_consumption_at(completed + Duration::milliseconds(30_001)),
        Err(DispatchBlock::AccountStale)
    );
    candidate
        .record_owner_receipt(OwnerReceipt::Accepted)
        .unwrap();
    assert!(matches!(
        candidate.prepare_dispatch(start() + Duration::minutes(1)),
        Err(DispatchBlock::AlreadyConsumed)
    ));

    let completed = start() + Duration::minutes(1);
    let mut candidate = qualified(&fresh_pack(completed), start(), completed)
        .into_candidate()
        .unwrap();
    assert!(candidate
        .prepare_dispatch(completed + Duration::milliseconds(30_000))
        .is_ok());
    let mut stale = qualified(&fresh_pack(completed), start(), completed)
        .into_candidate()
        .unwrap();
    assert!(matches!(
        stale.prepare_dispatch(completed + Duration::milliseconds(30_001)),
        Err(DispatchBlock::AccountStale)
    ));
    assert!(matches!(
        stale.prepare_dispatch(completed),
        Err(DispatchBlock::ClockReversed)
    ));
    assert!(matches!(
        stale.prepare_dispatch(completed + Duration::seconds(31)),
        Err(DispatchBlock::AccountStale)
    ));
    let mut expired = qualified(&fresh_pack(completed), start(), completed)
        .into_candidate()
        .unwrap();
    assert!(matches!(
        expired.prepare_dispatch(at(start().date_naive(), 15, 30)),
        Err(DispatchBlock::Expired)
    ));
    assert!(matches!(
        expired.prepare_dispatch(at(start().date_naive(), 15, 29)),
        Err(DispatchBlock::ClockReversed)
    ));
    assert!(matches!(
        expired.prepare_dispatch(at(start().date_naive(), 15, 31)),
        Err(DispatchBlock::Expired)
    ));
}
#[test]
fn fake_single_owner_deduplicates_stable_business_identity_and_unknown_never_retries() {
    struct FakeOwner {
        seen: std::collections::BTreeSet<String>,
        physical_attempts: usize,
        result: OwnerReceipt,
    }
    impl FakeOwner {
        fn consume(&mut self, request: &PreparedDispatch, now: Clock) -> OwnerReceipt {
            if request.validate_consumption_at(now).is_err() {
                return OwnerReceipt::Rejected;
            }
            assert!(!request.retry_authorized());
            if !self.seen.insert(request.occurrence_identity().into()) {
                return OwnerReceipt::Duplicate;
            }
            self.physical_attempts += 1;
            self.result
        }
    }
    let completed = start() + Duration::minutes(1);
    let mut owner = FakeOwner {
        seen: Default::default(),
        physical_attempts: 0,
        result: OwnerReceipt::Accepted,
    };
    let mut first = qualified(&fresh_pack(completed), start(), completed)
        .into_candidate()
        .unwrap();
    let request = first.prepare_dispatch(completed).unwrap();
    use sha2::{Digest, Sha256};
    assert_eq!(
        hex::encode(Sha256::digest(request.source_canonical())),
        request.source_sha256()
    );
    let first_hash = request.source_sha256().to_owned();
    let outcome = owner.consume(request, completed);
    first.record_owner_receipt(outcome).unwrap();
    let later = completed + Duration::seconds(1);
    let mut replay = qualified(&fresh_pack(later), start(), later)
        .into_candidate()
        .unwrap();
    let request = replay.prepare_dispatch(later).unwrap();
    assert_ne!(request.source_sha256(), first_hash);
    let outcome = owner.consume(request, later);
    assert_eq!(outcome, OwnerReceipt::Duplicate);
    replay.record_owner_receipt(outcome).unwrap();
    assert_eq!(owner.physical_attempts, 1);

    let mut unknown = qualified(&fresh_pack(completed), start(), completed)
        .into_candidate()
        .unwrap();
    assert!(unknown.record_owner_receipt(OwnerReceipt::Unknown).is_err());
    unknown.prepare_dispatch(completed).unwrap();
    unknown.record_owner_receipt(OwnerReceipt::Unknown).unwrap();
    assert!(matches!(
        unknown.prepare_dispatch(later),
        Err(DispatchBlock::UnknownRequiresOwnerReconcile)
    ));
    assert!(unknown
        .record_owner_receipt(OwnerReceipt::Accepted)
        .is_err());
}
#[test]
fn producer_capabilities_and_receipts_cannot_be_deserialized_or_cloned() {
    trait NotDeserialize<A> {
        fn check() {}
    }
    impl<T: ?Sized> NotDeserialize<()> for T {}
    struct Deserializable;
    impl<T: serde::de::DeserializeOwned> NotDeserialize<Deserializable> for T {}
    let _ = <DispatchCandidate as NotDeserialize<_>>::check;
    let _ = <PreparedDispatch as NotDeserialize<_>>::check;
    let _ = <ProductionReceipt as NotDeserialize<_>>::check;
    trait NotClone<A> {
        fn check() {}
    }
    impl<T: ?Sized> NotClone<()> for T {}
    struct Clonable;
    impl<T: Clone> NotClone<Clonable> for T {}
    let _ = <DispatchCandidate as NotClone<_>>::check;
    let _ = <PreparedDispatch as NotClone<_>>::check;
}

#[test]
fn database_observation_finishes_with_real_clock_and_cannot_revive_a_serialized_source() {
    let completed = start() + Duration::minutes(1);
    let production = produce_observed(&fresh_pack(completed), start(), completed).unwrap();
    let finished =
        finish_database_observation(production, at(start().date_naive(), 15, 30)).unwrap();
    assert_eq!(finished.receipt().state, ProductionState::Expired);
    assert_eq!(finished.receipt().preview.state, State::Expired);
    assert_eq!(finished.receipt().elapsed_ms, 28 * 60 * 1_000);
    assert!(finished.into_candidate().is_none());
    let qualified = qualified(&fresh_pack(completed), start(), completed);
    assert!(finish_database_observation(qualified, completed).is_err());
}

#[test]
fn admitted_daily_close_remains_partial_authority_and_rejects_price_date_or_time_substitution() {
    use crate::data_gateway::{AdmittedDailyBars, BatchEvidence};
    use crate::data_provider::{AdjustType, KlineData};
    let completed = start() + Duration::minutes(1);
    let bar = KlineData {
        date: completed.date_naive(),
        open: 8.,
        high: 8.,
        low: 8.,
        close: 8.,
        volume: 100.,
        amount: 800.,
        pct_chg: 0.,
        intraday_price: None,
        settled: true,
        pe_ratio: None,
        pb_ratio: None,
        turnover_rate: None,
        market_cap: None,
        circulating_cap: None,
        eps: None,
        roe: None,
        revenue_yoy: None,
        net_profit_yoy: None,
        gross_margin: None,
        net_margin: None,
        sharpe_ratio: None,
        financials_history: None,
        valuation_history: None,
        consensus: None,
        industry: None,
        is_limit_up: false,
        is_limit_down: false,
        is_suspended: false,
        adjust: AdjustType::None,
    };
    let evidence = BatchEvidence {
        provider: crate::market_domain::ProviderId::Tdx,
        source: "TEST_CODE_final_close".into(),
        source_at: Some("2026-09-28T15:00:00+08:00".into()),
        observed_at: completed.to_rfc3339(),
        batch_id: "TEST_CODE_close_batch".into(),
    };
    let admitted = |records, evidence| {
        AdmittedDailyBars::from_test_fixture("TEST_CODE_600396", records, evidence).unwrap()
    };
    let close =
        observe_admitted_close(&admitted(vec![bar.clone()], evidence.clone()), completed).unwrap();
    assert_eq!(close.close_micro_cny, 8_000_000);
    assert_eq!(close.instrument, "TEST_CODE_600396");
    assert!(close.authority.contains("NotRealSellDispatchAuthority"));
    assert_eq!(
        close.source_at.as_deref(),
        Some("2026-09-28T15:00:00+08:00")
    );
    for case in 0..7 {
        let mut changed_bar = bar.clone();
        let mut changed_evidence = evidence.clone();
        match case {
            0 => changed_bar.settled = false,
            1 => changed_bar.adjust = AdjustType::Qfq,
            2 => {
                changed_bar.date =
                    crate::calendar::verified_prev_a_share_trading_day(changed_bar.date).unwrap()
            }
            3 => changed_evidence.source_at = Some("2026-09-28T14:59:59+08:00".into()),
            4 => {
                changed_evidence.observed_at = (completed + Duration::milliseconds(1)).to_rfc3339()
            }
            5 => {
                changed_evidence.source_at =
                    Some((completed + Duration::milliseconds(1)).to_rfc3339())
            }
            6 => changed_bar.close = f64::NAN,
            _ => unreachable!(),
        }
        assert!(
            observe_admitted_close(&admitted(vec![changed_bar], changed_evidence), completed)
                .is_err(),
            "case={case}"
        );
    }
    assert!(
        observe_admitted_close(&admitted(vec![bar.clone(), bar], evidence), completed).is_err()
    );
}
