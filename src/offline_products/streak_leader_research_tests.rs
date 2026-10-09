use super::*;
use crate::performance::fee_policy::{
    FeeCoverage, FeeListingSegment, FeeMarket, FeeRate, FeeSecurityKind, QualifiedInstrument,
};
use chrono::{Duration, Utc};
fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 28).unwrap()
}
fn source(known_at: Clock) -> Source {
    Source {
        source: "test-independent".into(),
        revision: "test-v1".into(),
        sha256: "b".repeat(64),
        known_at,
        conflicted: false,
        invalidated: false,
    }
}
fn pack() -> EvidencePack {
    EvidencePack {
        schema: "streak-observed/v1".into(),
        days: vec![Day {
            date: date(),
            universe_source: Some(source(at(date(), 15, 0))),
            observations: vec!["B", "A", "C"]
                .into_iter()
                .map(|code| Observation {
                    instrument: code.into(),
                    streak: Some(2),
                    amount_micro_cny: Some(100000000),
                    close_micro_cny: Some(10_000_000),
                    source: source(at(date(), 15, 0)),
                    next_close_micro_cny: Some(10_500_000),
                    next_close_date: Some(verified_next_a_share_trading_day(date()).unwrap()),
                    next_close_source: Some(source(at(
                        verified_next_a_share_trading_day(date()).unwrap(),
                        15,
                        0,
                    ))),
                })
                .collect(),
        }],
    }
}
fn authority() -> QualifiedHistory {
    let scope = QualifiedInstrument::new(
        FeeMarket::Shanghai,
        FeeSecurityKind::AShareStock,
        FeeListingSegment::ShanghaiMainA,
    )
    .unwrap();
    let mut q = QualifiedHistory {
        days: BTreeSet::from([date()]),
        windows: BTreeMap::new(),
        fee: AShareFeePolicyV2::new(
            scope,
            FeeRate::new(3, 10000).unwrap(),
            5_000_000,
            FeeCoverage::initial_model(),
            "test-reviewed-2026",
        )
        .unwrap(),
        fee_source: source(at(date(), 14, 0)),
    };
    let entry = verified_next_a_share_trading_day(date()).unwrap();
    let exit = verified_next_a_share_trading_day(entry).unwrap();
    for (day, price) in [(entry, 10_000_000), (exit, 10_500_000)] {
        for code in ["A", "B", "C"] {
            let time = at(day, 9, 31);
            let utc = time.with_timezone(&Utc);
            q.windows.insert(
                (day, code.into()),
                HistoricalWindow {
                    record: WindowRecord {
                        version: fill::MODEL_VERSION.into(),
                        observation_id: format!("{code}-{day}"),
                        instrument_code: code.into(),
                        session_date: day,
                        source_at: utc,
                        observed_at: utc,
                        fresh_through: utc + Duration::seconds(5),
                        source_reference: "independent-window".into(),
                        facts_contract: "qualified-test-v1".into(),
                        facts_batch_id: "facts-batch".into(),
                        facts_source: "lifecycle-source".into(),
                        facts_source_at: time.to_rfc3339(),
                        facts_observed_at: time.to_rfc3339(),
                        fee_segment: "ShanghaiMainA".into(),
                        listed: true,
                        suspended: false,
                        tick_micro_cny: 10_000,
                        lower_micro_cny: 9_000_000,
                        upper_micro_cny: 11_000_000,
                        regime_version: "exact-test-main-a".into(),
                        price_micro_cny: price,
                        modeled_available_quantity: 300,
                    },
                    lifecycle_band_status: source(time),
                    executable_contra_liquidity: source(time),
                    corporate_action_scope: source(time),
                    queue_proven: true,
                },
            );
        }
    }
    q
}
fn end() -> Clock {
    at(NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(), 15, 0)
}
#[test]
fn public_dto_only_descriptive_ties_no_admission_and_no_zero_metrics() {
    let p = pack();
    let r = research_observed(&p, Policy::default(), end()).unwrap();
    assert_eq!(
        r.rows
            .iter()
            .map(|r| (&*r.instrument, r.observed_rank))
            .collect::<Vec<_>>(),
        vec![("A", Some(1)), ("B", Some(2)), ("C", Some(3))]
    );
    assert_eq!(r.denominator.qualified_days, 0);
    assert_eq!(r.denominator.picks, 0);
    assert!(r.modeled_win_rate.is_none());
    assert!(r.rows.iter().all(|r| r.entry_shares == 0 && !r.selected));
    assert!(r.authority.contains("PIT NotCertified"));
    let mut value = serde_json::to_value(p).unwrap();
    value["qualified"] = true.into();
    assert!(serde_json::from_value::<EvidencePack>(value).is_err());
}
#[test]
fn qualified_ranking_caps_and_future_inputs_excluded() {
    let mut p = pack();
    p.days[0].observations[0].streak = Some(3);
    p.days[0].observations[1].source.known_at += Duration::seconds(1);
    let r = research(
        &p,
        Policy {
            max_picks: 1,
            ..Policy::default()
        },
        end(),
        Some(&authority()),
    )
    .unwrap();
    assert_eq!(r.denominator.picks, 1);
    assert!(
        r.rows
            .iter()
            .find(|r| r.instrument == "B")
            .unwrap()
            .selected
    );
    assert_eq!(
        r.rows
            .iter()
            .find(|r| r.instrument == "A")
            .unwrap()
            .observed_rank,
        None
    );
    assert_eq!(r.denominator.decision_days, 1);
    assert_eq!(r.denominator.qualified_days, 1);
    assert_eq!(r.denominator.executable_entries, 1);
    assert_eq!(r.denominator.closed_trades, 1);
}
#[test]
fn no_signal_close_fill_and_unknown_contra_cannot_fill() {
    let p = pack();
    let mut q = authority();
    q.windows.clear();
    let r = research(&p, Policy::default(), end(), Some(&q)).unwrap();
    assert_eq!(r.denominator.picks, 3);
    assert_eq!(r.denominator.executable_entries, 0);
    assert_eq!(r.denominator.unknown_entries, 3);
    assert!(r.modeled_win_rate.is_none());
    let r = research(&p, Policy::default(), at(date(), 15, 0), Some(&authority())).unwrap();
    assert_eq!(r.denominator.executable_entries, 0);
    assert!(r.rows.iter().all(|r| r.entry_state.contains("not known")));
}
#[test]
fn upper_limit_queue_and_missing_lifecycle_refuse_fill() {
    let entry = verified_next_a_share_trading_day(date()).unwrap();
    let mut q = authority();
    let w = q.windows.get_mut(&(entry, "A".into())).unwrap();
    w.record.price_micro_cny = w.record.upper_micro_cny;
    w.queue_proven = false;
    let w = q.windows.get_mut(&(entry, "B".into())).unwrap();
    w.lifecycle_band_status.conflicted = true;
    let r = research(&pack(), Policy::default(), end(), Some(&q)).unwrap();
    assert!(r.rows[0].entry_state.contains("queue unknown"));
    assert!(r.rows[1].entry_state.contains("Unavailable"));
    assert_eq!(r.denominator.executable_entries, 1);
}
#[test]
fn partial_volume_t1_and_missing_exit_censored_not_carried_forward() {
    let entry = verified_next_a_share_trading_day(date()).unwrap();
    let exit = verified_next_a_share_trading_day(entry).unwrap();
    let mut q = authority();
    q.windows
        .get_mut(&(entry, "A".into()))
        .unwrap()
        .record
        .modeled_available_quantity = 250;
    q.windows
        .get_mut(&(exit, "A".into()))
        .unwrap()
        .record
        .modeled_available_quantity = 150;
    q.windows.remove(&(exit, "B".into()));
    q.windows
        .get_mut(&(exit, "C".into()))
        .unwrap()
        .record
        .suspended = true;
    let r = research(
        &pack(),
        Policy {
            shares: 300,
            ..Policy::default()
        },
        end(),
        Some(&q),
    )
    .unwrap();
    let a = &r.rows[0];
    assert_eq!(
        (a.entry_shares, a.closed_shares, a.censored_shares),
        (200, 100, 100)
    );
    assert_eq!(a.entry_state, "Partial");
    assert!(a.exit_session.unwrap() > a.entry_session.unwrap());
    assert_eq!(r.denominator.censored_entries, 3);
    assert_eq!(r.denominator.closed_trades, 0);
    assert!(r.modeled_win_rate.is_none());
    assert!(r.rows[1].exit_state.starts_with("Censored"));
    assert!(r.rows[2].exit_state.contains("Suspended"));
}
#[test]
fn fees_are_explicit_dated_per_fill_and_settled_unavailable() {
    let q = authority();
    let r = research(&pack(), Policy::default(), end(), Some(&q)).unwrap();
    let a = &r.rows[0];
    assert_eq!(a.buy_commission_micro_cny, Some(5_000_000));
    assert_eq!(a.sell_commission_micro_cny, Some(5_000_000));
    assert_eq!(a.sell_stamp_micro_cny, Some(525_000));
    assert_eq!(a.modeled_covered_net_micro_cny, Some(39_475_000));
    assert!(a.actual_settled_net.is_none());
    assert!(r.fee_scope.contains("transfer/other excluded"));
    assert_eq!(r.fee_hash, Some(q.fee.descriptor_hash()));
    assert_eq!(r.modeled_win_rate, Some(1.));
}
#[test]
fn holiday_next_session_and_empty_unknown_day() {
    let d = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
    assert_eq!(
        verified_next_a_share_trading_day(d).unwrap(),
        NaiveDate::from_ymd_opt(2026, 10, 8).unwrap()
    );
    let mut holiday_pack = pack();
    holiday_pack.days[0].date = d;
    let holiday_study = research_observed(&holiday_pack, Policy::default(), at(d, 15, 0)).unwrap();
    assert_eq!(
        holiday_study.rows[0].entry_session,
        Some(NaiveDate::from_ymd_opt(2026, 10, 8).unwrap())
    );
    let p = EvidencePack {
        schema: "streak-observed/v1".into(),
        days: vec![Day {
            date: d,
            universe_source: None,
            observations: vec![],
        }],
    };
    let r = research_observed(&p, Policy::default(), at(d, 15, 0)).unwrap();
    assert_eq!(r.denominator.decision_days, 1);
    assert_eq!(r.denominator.qualified_days, 0);
    assert_eq!(r.unavailable_days.len(), 1);
    assert!(r.modeled_win_rate.is_none());
}
#[test]
fn one_window_expiry_and_lower_queue_unknown() {
    let mut q = authority();
    let entry = verified_next_a_share_trading_day(date()).unwrap();
    let exit = verified_next_a_share_trading_day(entry).unwrap();
    let a = q.windows.get_mut(&(entry, "A".into())).unwrap();
    a.record.observed_at = at(entry, 9, 35).with_timezone(&Utc);
    let b = q.windows.get_mut(&(exit, "B".into())).unwrap();
    b.record.price_micro_cny = b.record.lower_micro_cny;
    b.queue_proven = false;
    let r = research(&pack(), Policy::default(), end(), Some(&q)).unwrap();
    assert!(r.rows[0].entry_state.contains("expiry"));
    assert!(r.rows[1].exit_state.contains("queue unknown"));
}
#[test]
fn stable_hashes_export_scopes_and_safe_strings() {
    let p = pack();
    let a = research_observed(&p, Policy::default(), end()).unwrap();
    let b = research_observed(&p, Policy::default(), end()).unwrap();
    assert_eq!(
        serde_json::to_vec(&a).unwrap(),
        serde_json::to_vec(&b).unwrap()
    );
    let changed = research_observed(
        &p,
        Policy {
            max_picks: 2,
            ..Policy::default()
        },
        end(),
    )
    .unwrap();
    assert_ne!(a.strategy_hash, changed.strategy_hash);
    assert_eq!(a.input_hash, changed.input_hash);
    let mut r = a;
    r.rows[0].instrument = "=HYPERLINK(\"bad\")|<script>".into();
    assert!(r.csv().contains("'=HYPERLINK"));
    assert!(!r.markdown().contains("<script>"));
    r.attach_checked_observation(serde_json::json!({"capture_sha256":"c","records":[{"schema":"magic.market.bar","record_json":{"close":10}}]}));
    assert!(r.csv().contains("checked_store_raw_observation"));
    assert_eq!(r.denominator.qualified_days, 0);
    assert!(r.modeled_win_rate.is_none());
}

#[test]
fn accepted_huge_commissions_censor_failed_economics_without_zero_headline() {
    let mut q = authority();
    q.fee = AShareFeePolicyV2::new(
        q.fee.scope(),
        FeeRate::new(3, 10000).unwrap(),
        5_000_000_000_000_000_000,
        FeeCoverage::initial_model(),
        "test-huge-fee-boundary",
    )
    .unwrap();
    let r = research(&pack(), Policy::default(), end(), Some(&q)).unwrap();
    assert_eq!(r.denominator.picks, 3);
    assert_eq!(r.denominator.executable_entries, 3);
    assert_eq!(r.denominator.closed_trades, 0);
    assert_eq!(r.denominator.censored_entries, 3);
    assert_eq!(r.denominator.invalid_economics, 3);
    assert!(r.modeled_win_rate.is_none());
    assert!(r.modeled_covered_net_micro_cny.is_none());
    assert!(r.metric_failure.is_some());
    for row in &r.rows {
        assert_eq!(
            (row.entry_shares, row.closed_shares, row.censored_shares),
            (100, 0, 100)
        );
        assert!(row.economic_failure.is_some());
        assert!(row.exit_state.contains("economic net unavailable"));
        assert!(row.modeled_covered_net_micro_cny.is_none());
        assert!(row.exit_price_micro_cny.is_none());
    }
    let json = serde_json::to_value(&r).unwrap();
    assert!(json["modeled_win_rate"].is_null());
    assert!(json["modeled_covered_net_micro_cny"].is_null());
    assert!(r.markdown().contains("经济计算失败"));
    assert!(r.csv().contains("invalid_economics"));
}
