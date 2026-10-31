use magic_market_core::{
    verify_auction_conformance, AssetClass, AuctionConformancePolicy, AuctionSnapshot, DataBatch,
    DataStatus, Exchange, InstrumentId, IsoDate, Money, NonEmptyText, Price, Provenance,
    ProviderId, Quantity, Ratio, RatioUnit,
};
use std::time::Duration;

fn instrument(code: &str) -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity).unwrap()
}

fn auction(
    instrument: InstrumentId,
    provider: ProviderId,
    batch_id: &str,
    source_at: Option<&str>,
    available: bool,
) -> AuctionSnapshot {
    auction_observed_at(
        instrument,
        provider,
        batch_id,
        source_at,
        "2026-07-27T09:25:01+08:00",
        available,
    )
}

fn auction_observed_at(
    instrument: InstrumentId,
    provider: ProviderId,
    batch_id: &str,
    source_at: Option<&str>,
    observed_at: &str,
    available: bool,
) -> AuctionSnapshot {
    AuctionSnapshot::new(
        instrument,
        available.then(|| "测试股份".into()),
        available.then(|| Price::new(10.0).unwrap()),
        available.then(|| Price::new(9.9).unwrap()),
        available.then(|| Ratio::new(1.01, RatioUnit::Percent).unwrap()),
        available.then(|| Quantity::new(1_000.0).unwrap()),
        available.then(|| Money::new(10_000.0).unwrap()),
        available.then(|| Quantity::new(200.0).unwrap()),
        available.then(|| Quantity::new(300.0).unwrap()),
        available.then(|| Ratio::new(1.2, RatioUnit::Decimal).unwrap()),
        if available {
            DataStatus::Available
        } else {
            DataStatus::Unavailable
        },
        source_at.map(str::to_owned),
        observed_at,
        provider,
        batch_id,
    )
    .unwrap()
}

fn policy(provider: ProviderId) -> AuctionConformancePolicy {
    AuctionConformancePolicy::new(
        provider,
        NonEmptyText::new("licensed-level2").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
        Duration::from_secs(5),
    )
    .unwrap()
}

fn provenance(batch_id: &str, source_at: Option<&str>) -> Provenance {
    let base = Provenance::new("licensed-level2", "2026-07-27T09:25:01+08:00")
        .unwrap()
        .with_batch_id(batch_id)
        .unwrap();
    match source_at {
        Some(source_at) => base.with_source_at(source_at).unwrap(),
        None => base,
    }
}

#[test]
fn complete_authorized_auction_batch_passes_the_common_contract() {
    let requested = [instrument("600000"), instrument("600001")];
    let source_at = "2026-07-27T09:25:00+08:00";
    let batch_id = "licensed:auction:1";
    let batch = DataBatch::strict(
        requested
            .iter()
            .cloned()
            .map(|item| {
                auction(
                    item,
                    ProviderId::LocalTerminal,
                    batch_id,
                    Some(source_at),
                    true,
                )
            })
            .collect(),
        provenance(batch_id, Some(source_at)),
    );

    verify_auction_conformance(&requested, policy(ProviderId::LocalTerminal), &batch).unwrap();
}

#[test]
fn contract_rejects_partial_fields_and_missing_source_time() {
    let requested = [instrument("600000")];
    let batch_id = "diagnostic:auction:1";
    let partial = DataBatch::best_effort(
        vec![auction(
            requested[0].clone(),
            ProviderId::Tencent,
            batch_id,
            None,
            false,
        )],
        provenance(batch_id, None),
        vec!["public response has no unmatched auction queues".into()],
    )
    .unwrap();

    let error =
        verify_auction_conformance(&requested, policy(ProviderId::Tencent), &partial).unwrap_err();
    assert!(error.to_string().contains("incomplete"));
}

#[test]
fn contract_rejects_identity_provider_batch_and_time_disagreement() {
    let requested = [instrument("600000")];
    let source_at = "2026-07-27T09:25:00+08:00";
    let batch_id = "licensed:auction:1";

    let wrong_identity = DataBatch::strict(
        vec![auction(
            instrument("600001"),
            ProviderId::LocalTerminal,
            batch_id,
            Some(source_at),
            true,
        )],
        provenance(batch_id, Some(source_at)),
    );
    assert!(verify_auction_conformance(
        &requested,
        policy(ProviderId::LocalTerminal),
        &wrong_identity
    )
    .unwrap_err()
    .to_string()
    .contains("identity"));

    let wrong_provider = DataBatch::strict(
        vec![auction(
            requested[0].clone(),
            ProviderId::Tencent,
            batch_id,
            Some(source_at),
            true,
        )],
        provenance(batch_id, Some(source_at)),
    );
    assert!(verify_auction_conformance(
        &requested,
        policy(ProviderId::LocalTerminal),
        &wrong_provider
    )
    .unwrap_err()
    .to_string()
    .contains("provider"));

    let wrong_batch = DataBatch::strict(
        vec![auction(
            requested[0].clone(),
            ProviderId::LocalTerminal,
            "licensed:auction:other",
            Some(source_at),
            true,
        )],
        provenance(batch_id, Some(source_at)),
    );
    assert!(verify_auction_conformance(
        &requested,
        policy(ProviderId::LocalTerminal),
        &wrong_batch
    )
    .unwrap_err()
    .to_string()
    .contains("batch"));

    let wrong_time = DataBatch::strict(
        vec![auction(
            requested[0].clone(),
            ProviderId::LocalTerminal,
            batch_id,
            Some("2026-07-27T09:24:59+08:00"),
            true,
        )],
        provenance(batch_id, Some(source_at)),
    );
    assert!(
        verify_auction_conformance(&requested, policy(ProviderId::LocalTerminal), &wrong_time)
            .unwrap_err()
            .to_string()
            .contains("source time")
    );

    let wrong_source = DataBatch::strict(
        vec![auction(
            requested[0].clone(),
            ProviderId::LocalTerminal,
            batch_id,
            Some(source_at),
            true,
        )],
        Provenance::new("wrong-level2-source", "2026-07-27T09:25:01+08:00")
            .unwrap()
            .with_source_at(source_at)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap(),
    );
    assert!(verify_auction_conformance(
        &requested,
        policy(ProviderId::LocalTerminal),
        &wrong_source
    )
    .unwrap_err()
    .to_string()
    .contains("batch source"));
}

#[test]
fn request_must_be_nonempty_duplicate_free_and_exact_cardinality() {
    let batch_id = "licensed:auction:1";
    let source_at = "2026-07-27T09:25:00+08:00";
    let empty =
        DataBatch::<AuctionSnapshot>::strict(Vec::new(), provenance(batch_id, Some(source_at)));
    assert!(
        verify_auction_conformance(&[], policy(ProviderId::LocalTerminal), &empty)
            .unwrap_err()
            .to_string()
            .contains("empty")
    );

    let requested = [instrument("600000"), instrument("600000")];
    let one = DataBatch::strict(
        vec![auction(
            requested[0].clone(),
            ProviderId::LocalTerminal,
            batch_id,
            Some(source_at),
            true,
        )],
        provenance(batch_id, Some(source_at)),
    );
    assert!(
        verify_auction_conformance(&requested, policy(ProviderId::LocalTerminal), &one)
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
}

#[test]
fn timestamp_policy_rejects_zero_malformed_future_stale_and_observation_mismatch() {
    assert!(AuctionConformancePolicy::new(
        ProviderId::LocalTerminal,
        NonEmptyText::new("licensed-level2").unwrap(),
        IsoDate::new("2026-07-27").unwrap(),
        Duration::ZERO,
    )
    .is_err());

    let requested = [instrument("600000")];
    let batch_id = "licensed:auction:time";
    for (source_at, observed_at, expected) in [
        (
            "not-a-time",
            "2026-07-27T09:25:01+08:00",
            "source time is malformed",
        ),
        (
            "2026-07-27",
            "2026-07-27T09:25:01+08:00",
            "source time is malformed",
        ),
        (
            "2026-07-27T09:25:00",
            "2026-07-27T09:25:01+08:00",
            "source time is malformed",
        ),
        (
            "2026-07-27T00:00:00Z",
            "2026-07-27",
            "observation time is malformed",
        ),
        (
            "2026-07-27T09:25:02+08:00",
            "2026-07-27T09:25:01+08:00",
            "source time is in the future",
        ),
        (
            "2026-07-27T09:24:55.999+08:00",
            "2026-07-27T09:25:01+08:00",
            "source age 5001ms",
        ),
    ] {
        let batch = DataBatch::strict(
            vec![auction_observed_at(
                requested[0].clone(),
                ProviderId::LocalTerminal,
                batch_id,
                Some(source_at),
                observed_at,
                true,
            )],
            Provenance::new("licensed-level2", observed_at)
                .unwrap()
                .with_source_at(source_at)
                .unwrap()
                .with_batch_id(batch_id)
                .unwrap(),
        );
        assert!(
            verify_auction_conformance(&requested, policy(ProviderId::LocalTerminal), &batch)
                .unwrap_err()
                .to_string()
                .contains(expected)
        );
    }

    let source_at = "2026-07-27T09:25:00+08:00";
    let mismatch = DataBatch::strict(
        vec![auction_observed_at(
            requested[0].clone(),
            ProviderId::LocalTerminal,
            batch_id,
            Some(source_at),
            "2026-07-27T09:25:02+08:00",
            true,
        )],
        provenance(batch_id, Some(source_at)),
    );
    assert!(
        verify_auction_conformance(&requested, policy(ProviderId::LocalTerminal), &mismatch)
            .unwrap_err()
            .to_string()
            .contains("observation time")
    );
}

#[test]
fn source_time_must_be_in_the_explicit_china_opening_auction_window() {
    let requested = [instrument("600000")];
    let batch_id = "licensed:auction:session";
    for (source_at, observed_at) in [
        ("2026-07-27T09:14:59+08:00", "2026-07-27T09:15:00+08:00"),
        (
            "2026-07-27T09:25:00.999999999+08:00",
            "2026-07-27T09:25:01+08:00",
        ),
        ("2026-07-27T09:25:01+08:00", "2026-07-27T09:25:02+08:00"),
        ("2026-07-27T13:00:00+08:00", "2026-07-27T13:00:01+08:00"),
        ("2026-07-27T15:00:00+08:00", "2026-07-27T15:00:01+08:00"),
        ("2026-07-28T09:25:00+08:00", "2026-07-28T09:25:01+08:00"),
        ("2026-07-27T09:25:00Z", "2026-07-27T09:25:01Z"),
    ] {
        let batch = DataBatch::strict(
            vec![auction_observed_at(
                requested[0].clone(),
                ProviderId::LocalTerminal,
                batch_id,
                Some(source_at),
                observed_at,
                true,
            )],
            Provenance::new("licensed-level2", observed_at)
                .unwrap()
                .with_source_at(source_at)
                .unwrap()
                .with_batch_id(batch_id)
                .unwrap(),
        );
        assert!(
            verify_auction_conformance(&requested, policy(ProviderId::LocalTerminal), &batch)
                .unwrap_err()
                .to_string()
                .contains("auction")
        );
    }
}

#[test]
fn contract_rejects_short_duplicate_and_field_incomplete_responses() {
    let requested = [instrument("600000"), instrument("600001")];
    let source_at = "2026-07-27T09:25:00+08:00";
    let batch_id = "licensed:auction:shape";

    let short = DataBatch::strict(
        vec![auction(
            requested[0].clone(),
            ProviderId::LocalTerminal,
            batch_id,
            Some(source_at),
            true,
        )],
        provenance(batch_id, Some(source_at)),
    );
    assert!(
        verify_auction_conformance(&requested, policy(ProviderId::LocalTerminal), &short)
            .unwrap_err()
            .to_string()
            .contains("cardinality")
    );

    let duplicate = DataBatch::strict(
        vec![
            auction(
                requested[0].clone(),
                ProviderId::LocalTerminal,
                batch_id,
                Some(source_at),
                true,
            ),
            auction(
                requested[0].clone(),
                ProviderId::LocalTerminal,
                batch_id,
                Some(source_at),
                true,
            ),
        ],
        provenance(batch_id, Some(source_at)),
    );
    assert!(
        verify_auction_conformance(&requested, policy(ProviderId::LocalTerminal), &duplicate)
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );

    let incomplete = DataBatch::strict(
        vec![auction(
            requested[0].clone(),
            ProviderId::LocalTerminal,
            batch_id,
            Some(source_at),
            false,
        )],
        provenance(batch_id, Some(source_at)),
    );
    assert!(verify_auction_conformance(
        &requested[..1],
        policy(ProviderId::LocalTerminal),
        &incomplete
    )
    .unwrap_err()
    .to_string()
    .contains("incomplete for the authorized contract"));
}
