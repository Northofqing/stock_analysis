use super::*;
use chrono::Duration;
fn clock() -> Clock {
    super::super::shanghai_clock("2026-09-28T15:05:00+08:00").unwrap()
}
fn source() -> Source {
    Source {
        source: "independent-test".into(),
        revision: "test/v1".into(),
        sha256: "a".repeat(64),
        known_at: clock() - Duration::seconds(5),
        conflicted: false,
        invalidated: false,
    }
}
pub(super) fn fixture(price: f64, n: usize) -> EvidencePack {
    let now = clock();
    let day = now.date_naive();
    let acquired = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
    let mut bars = vec![];
    let mut date = day;
    for _ in 0..n {
        bars.push(Bar {
            date,
            open: price,
            high: price + 0.5,
            low: price - 0.5,
            close: price,
            volume: 1000.,
        });
        date = crate::calendar::verified_prev_a_share_trading_day(date).unwrap();
    }
    EvidencePack {
        schema: "sell-evidence-observed/v1".into(),
        account: Some(Account {
            account_ref: "real-test".into(),
            ownership: "self".into(),
            environment: "real".into(),
            captured_at: now,
            observed_at: now,
            complete: true,
            confirmed_empty: false,
            source: source(),
        }),
        lots: vec![Lot {
            account_ref: "real-test".into(),
            instrument: "TEST".into(),
            lot_id: Some("lot-old".into()),
            acquired: Some(acquired),
            sellable_from: Some(verified_next_a_share_trading_day(acquired).unwrap()),
            total: 200,
            sellable: Some(200),
            reserved: Some(0),
            cost_micro_cny: Some(10_000_000),
            allocated_buy_fee_micro_cny: Some(5_000_000),
            sell_fees: Some(SellFees {
                shares: 200,
                commission_micro_cny: Some(5_000_000),
                stamp_micro_cny: Some(1_000_000),
                transfer_micro_cny: Some(0),
                other_micro_cny: Some(0),
                source: source(),
            }),
            source: source(),
        }],
        securities: vec![Security {
            instrument: "TEST".into(),
            board: "SSE.MainA".into(),
            quantity: QuantityContract {
                minimum: 100,
                step: 100,
                max_per_order: 1_000_000,
                whole_remaining_odd_lot: true,
                source: source(),
            },
            date: day,
            close_micro_cny: Some((price * 1e6) as i64),
            close_finalized: true,
            close_source: source(),
            listed: Some(true),
            suspended_at_close: Some(false),
            status_source: source(),
            bars_source: source(),
            adjustment: "unadjusted".into(),
            bars,
        }],
    }
}
fn qualified(pack: &EvidencePack) -> Preview {
    evaluate(pack, clock(), Some(&QualifiedSource))
}
#[test]
fn rule_trigger_hold_unknown_and_no_buy_veto() {
    let sell = qualified(&fixture(8., 60));
    assert_eq!(sell.state, State::Candidate);
    assert_eq!(sell.rows[0].suggested_shares, 200);
    assert!(sell.rows[0].reason.contains("止损"));
    assert_eq!(qualified(&fixture(10., 60)).state, State::NoSuggestion);
    assert_eq!(qualified(&fixture(10., 20)).state, State::Unavailable);
    let small = qualified(&fixture(8., 1));
    assert_eq!(small.state, State::Candidate);
    assert!(small.rows[0].indicator_scope.contains("fallback=true"));
    // No buy/regime field exists: the deterministic SELL path has no buy veto.
    assert!(sell.semantics.contains("单位差异"));
}
#[test]
fn capture_and_observation_exact_freshness_and_future() {
    for capture in [true, false] {
        for (ms, expected) in [
            (30_000, State::Candidate),
            (30_001, State::Unavailable),
            (-1, State::Unavailable),
        ] {
            let mut p = fixture(8., 60);
            let a = p.account.as_mut().unwrap();
            if capture {
                a.captured_at = clock() - Duration::milliseconds(ms);
            } else {
                a.observed_at = clock() - Duration::milliseconds(ms);
                a.captured_at = a.observed_at;
            }
            assert_eq!(qualified(&p).state, expected, "capture={capture} ms={ms}");
        }
    }
}
#[test]
fn temporal_window_exact_close_and_sticky_expiry() {
    let p = fixture(8., 60);
    for (hour, min, sec, expected) in [
        (14, 59, 59, State::Unavailable),
        (15, 0, 0, State::Candidate),
        (15, 29, 59, State::Candidate),
        (15, 30, 0, State::Expired),
    ] {
        let now = at(clock().date_naive(), hour, min) + Duration::seconds(sec);
        let mut p = p.clone();
        let a = p.account.as_mut().unwrap();
        a.captured_at = now;
        a.observed_at = now;
        a.source.known_at = now;
        for s in &mut p.securities {
            for source in [
                &mut s.close_source,
                &mut s.status_source,
                &mut s.quantity.source,
                &mut s.bars_source,
            ] {
                source.known_at = now;
            }
        }
        p.lots[0].source.known_at = now;
        p.lots[0].sell_fees.as_mut().unwrap().source.known_at = now;
        assert_eq!(evaluate(&p, now, Some(&QualifiedSource)).state, expected);
    }
    let mut r = qualified(&p);
    r.reinspect(at(clock().date_naive(), 15, 30)).unwrap();
    assert_eq!(r.state, State::Expired);
    assert_eq!(r.rows[0].suggested_shares, 0);
    assert!(r.reinspect(clock()).is_err());
    assert_eq!(r.state, State::Expired);
    assert_eq!(
        reinspect_imported(
            serde_json::from_value(serde_json::to_value(qualified(&p)).unwrap()).unwrap(),
            clock()
        )
        .unwrap()
        .state,
        State::Unavailable
    );
}
#[test]
fn today_locked_old_eligible_reservations_and_odd_lot() {
    let mut p = fixture(8., 60);
    let mut today = p.lots[0].clone();
    today.lot_id = Some("today".into());
    today.acquired = Some(clock().date_naive());
    today.sellable_from = Some(verified_next_a_share_trading_day(clock().date_naive()).unwrap());
    today.sellable = Some(0);
    p.lots.push(today);
    let r = qualified(&p);
    assert_eq!(r.state, State::Candidate);
    assert_eq!(r.rows.iter().map(|r| r.suggested_shares).sum::<u32>(), 200);
    assert!(r.rows.iter().any(|r| r.reason.contains("T+1")));
    p.lots[0].reserved = Some(100);
    p.lots[0].sell_fees.as_mut().unwrap().shares = 100;
    assert_eq!(qualified(&p).rows[0].suggested_shares, 100);
    p.lots.truncate(1);
    p.lots[0].total = 50;
    p.lots[0].sellable = Some(50);
    p.lots[0].reserved = Some(0);
    p.lots[0].sell_fees.as_mut().unwrap().shares = 50;
    assert_eq!(qualified(&p).rows[0].suggested_shares, 50);
    p.lots[0].reserved = Some(1);
    assert_eq!(qualified(&p).rows[0].suggested_shares, 0);
}
#[test]
fn missing_fields_and_bad_sources_never_hold() {
    let p = fixture(8., 60);
    for field in 0..12 {
        let mut p = p.clone();
        match field {
            0 => p.lots[0].cost_micro_cny = None,
            1 => p.lots[0].sell_fees = None,
            2 => p.lots[0].sellable = None,
            3 => p.lots[0].reserved = None,
            4 => p.securities[0].close_micro_cny = None,
            5 => p.securities[0].suspended_at_close = None,
            6 => p.securities[0].close_finalized = false,
            7 => p.securities[0].close_source.invalidated = true,
            8 => p.securities[0].status_source.conflicted = true,
            9 => p.securities[0].close_source.known_at = clock() + Duration::seconds(1),
            10 => p.securities[0].bars[0].close = f64::NAN,
            11 => p.securities[0].board = "BSE".into(),
            _ => unreachable!(),
        };
        assert_eq!(qualified(&p).state, State::Unavailable, "case={field}");
    }
    let mut p = p;
    p.securities[0].suspended_at_close = Some(true);
    assert_eq!(qualified(&p).state, State::NoSuggestion);
}
#[test]
fn absent_incomplete_and_confirmed_empty_distinct_no_dto_authority() {
    let mut p = fixture(8., 60);
    assert_eq!(preview_observed(&p, clock()).state, State::Unavailable);
    p.lots.clear();
    p.account.as_mut().unwrap().confirmed_empty = true;
    assert_eq!(qualified(&p).state, State::NoSuggestion);
    assert_eq!(
        qualified(&p).account_state,
        "confirmed_complete_empty_observed"
    );
    p.account.as_mut().unwrap().complete = false;
    assert_eq!(qualified(&p).state, State::Unavailable);
    p.account = None;
    assert_eq!(qualified(&p).account_state, "missing");
    let mut dto = serde_json::to_value(fixture(8., 60)).unwrap();
    dto["qualified"] = true.into();
    assert!(serde_json::from_value::<EvidencePack>(dto).is_err());
}
#[test]
fn markdown_escapes_sources_and_human_record_never_settlement() {
    let mut p = fixture(8., 60);
    p.lots[0].instrument = "<script>|[click](https://bad)".into();
    let r = preview_observed(&p, clock());
    let md = r.markdown();
    assert!(!md.contains("<script>"));
    assert!(md.contains("&lt;script&gt;&#124;"));
    assert_eq!(r.handling_template()["not_settlement"], true);
}
#[test]
fn deterministic_indicator_seam_matches_live_trend_and_boll() {
    let p = fixture(10., 35);
    let s = &p.securities[0];
    let i = indicators(s, 10_000_000).unwrap();
    assert_eq!(i.ma5, Some(10.));
    assert_eq!(i.ma20, Some(10.));
    assert_eq!(i.ma60, Some(10.));
    assert_eq!(i.atr, Some(1.));
    assert!(i.scope.contains("MA60 substitutes MA20=true"));
    assert!(i.complete);
}
#[test]
fn database_reader_preserves_total_only_and_source_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("snapshot.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE user_position_snapshot(snapshot_id TEXT,effective_at TEXT,confirmed_at TEXT,source TEXT,evidence_sha256 TEXT,confirm_empty INTEGER,item_count INTEGER);CREATE TABLE user_position_snapshot_item(snapshot_id TEXT,code TEXT,quantity INTEGER,cost_price REAL);INSERT INTO user_position_snapshot VALUES('s','2026-09-28T15:05:00+08:00','2026-09-28T15:05:00+08:00','test','hash',0,1);INSERT INTO user_position_snapshot_item VALUES('s','TEST',300,10);").unwrap();
    drop(conn);
    let before = super::super::io::file_hash(&path).unwrap();
    let r = diagnose_database(&path, clock()).unwrap();
    assert_eq!(r.state, State::Unavailable);
    assert_eq!(r.account_state, "complete_aggregate_positions_observed");
    assert!(r.rows[0].missing.iter().any(|s| s.contains("lot_id")));
    assert_eq!(before, super::super::io::file_hash(&path).unwrap());
    assert!(!dir.path().join("snapshot.db-wal").exists());
    // A malformed aggregate quantity stays a diagnostic, never a substituted 0-share position.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute("UPDATE user_position_snapshot_item SET quantity=-1", [])
        .unwrap();
    drop(conn);
    let invalid_before = super::super::io::file_hash(&path).unwrap();
    let invalid = diagnose_database(&path, clock()).unwrap();
    assert!(invalid.rows.is_empty());
    assert_eq!(invalid.account_state, "incomplete_or_conflicting_snapshot");
    assert!(invalid.missing.iter().any(|s| s.contains("positive u32")));
    assert_eq!(invalid_before, super::super::io::file_hash(&path).unwrap());
}

#[test]
fn serialized_positive_report_cannot_reestablish_qualified_values_even_after_expiry() {
    let original = qualified(&fixture(8., 60));
    assert_eq!(original.state, State::Candidate);
    assert!(original.rows[0].net_scenario_pct.is_some());
    let bytes = serde_json::to_vec(&original).unwrap();
    for now in [clock(), at(clock().date_naive(), 15, 30)] {
        let forged: ImportedPreview = serde_json::from_slice(&bytes).unwrap();
        let imported = reinspect_imported(forged, now).unwrap();
        assert_eq!(
            imported.state,
            if now >= original.expires_at {
                State::Expired
            } else {
                State::Unavailable
            }
        );
        assert_eq!(imported.authority, "ImportedReport/NotAdmitted");
        assert_eq!(imported.rows[0].observed_total_shares, 200);
        assert_eq!(imported.rows[0].observed_close_micro_cny, Some(8_000_000));
        assert_eq!(imported.rows[0].suggested_shares, 0);
        assert!(imported.rows[0].eligible_sellable.is_none());
        assert!(imported.rows[0].reference_close_micro_cny.is_none());
        assert!(imported.rows[0].net_scenario_pct.is_none());
        assert!(imported.rows[0].covered_buy_fee_micro_cny.is_none());
        assert!(imported.rows[0].sell_fees.is_none());
        assert!(imported.markdown().contains("NotAdmitted"));
        // Every public render/serialization path sees only the masked result.
        let value = serde_json::to_value(&imported).unwrap();
        assert_eq!(value["authority"], "ImportedReport/NotAdmitted");
        assert!(value["rows"][0]["reference_close_micro_cny"].is_null());
        assert!(value["rows"][0]["net_scenario_pct"].is_null());
        assert!(value["rows"][0]["sell_fees"].is_null());
        assert!(!imported.markdown().starts_with("有卖出候选"));
        assert_eq!(imported.handling_template()["not_settlement"], true);
    }
    let mut trusted = original;
    trusted.reinspect(clock()).unwrap();
    assert_eq!(trusted.state, State::Candidate);
    assert!(trusted.rows[0].net_scenario_pct.is_some());
}
