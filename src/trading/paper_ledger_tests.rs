use super::*;
use crate::database::DatabaseManager;
use crate::trading::paper_trade::{Direction, PaperRiskContext, PaperSignal};
use chrono::{TimeZone, Utc};
use diesel::RunQueryDsl;

fn order(
    ledger: &PaperLedger<'_>,
    binding: &AccountBinding,
    id: &str,
    side: Direction,
    price: f64,
    at: chrono::DateTime<Utc>,
) -> ExecuteIntent {
    let view = ledger.read(binding).unwrap();
    ExecuteIntent {
        price_intent: PriceIntent::FixedSignalPriceV1,
        binding: binding.clone(),
        command_id: id.into(),
        expected_version: view.version,
        inventory_fingerprint: view.inventory_fingerprint().unwrap(),
        signal: PaperSignal {
            plan_id: format!("paper:{}:test:{id}", binding.epoch_id),
            code: "TEST_CODE_000001".into(),
            name: "fixture".into(),
            direction: side,
            price,
            quantity: 100,
            virtual_reason: "TEST_CODE_evidence".into(),
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            limit_up_price: Some(price * 1.1),
            limit_down_price: Some(price * 0.9),
            secondary_confirmed: false,
            quote_observed_at: at,
            risk_context: PaperRiskContext::new(
                crate::risk::action_gate::AccountMode::Normal,
                crate::monitor::data_mode::DataMode::Full,
            ),
        },
        quote_price: Money::from_cny(price).unwrap(),
        marks: vec![Mark {
            code: "TEST_CODE_000001".into(),
            price: Money::from_cny(price).unwrap(),
            observed_at: at,
            source: "TEST_CODE_realtime".into(),
        }],
    }
}

fn mark_close(
    ledger: &PaperLedger<'_>,
    binding: &AccountBinding,
    id: &str,
    price: f64,
    at: chrono::DateTime<Utc>,
) {
    let view = ledger.read(binding).unwrap();
    ledger
        .apply(PaperCommand::Mark(ValuationBatch {
            binding: binding.clone(),
            command_id: id.into(),
            expected_version: view.version,
            inventory_fingerprint: view.inventory_fingerprint().unwrap(),
            as_of: at,
            closing: true,
            marks: vec![Mark {
                code: "TEST_CODE_000001".into(),
                price: Money::from_cny(price).unwrap(),
                observed_at: at,
                source: "TEST_CODE_validated_daily_close".into(),
            }],
        }))
        .unwrap();
}

fn instant() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 14, 2, 0, 0).unwrap()
}

fn manifest() -> SeedManifest {
    SeedManifest {
        account_id: "TEST_CODE_ACCOUNT".into(),
        epoch_id: "TEST_CODE_EPOCH_V1".into(),
        command_id: "seed-v1".into(),
        cutover_at: instant(),
        account_effective_at: instant(),
        positions_effective_at: instant(),
        source_reference: "TEST_CODE_confirmed_snapshot".into(),
        source_hash: "a".repeat(64),
        approved_by: "TEST_CODE_explicit_approval".into(),
        cash: Money::from_cny(100_000.0).unwrap(),
        original_total: Money::from_cny(100_000.0).unwrap(),
        excluded_residual: None,
        lots: vec![],
        marks: vec![],
        policy: RiskPolicyV1::default(),
    }
}

#[test]
fn paper_ledger_seed_is_once_and_ignores_later_account_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_paper_seed.db");
    let seed = manifest();
    let binding = seed.binding().unwrap();
    {
        let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
        let ledger = PaperLedger::open(&db, &instant);
        assert!(matches!(ledger.read(&binding), Err(LedgerError::NotSeeded)));
        let first = ledger.apply(PaperCommand::Seed(seed.clone())).unwrap();
        assert!(!first.already_applied);
        let repeat = ledger.apply(PaperCommand::Seed(seed.clone())).unwrap();
        assert!(repeat.already_applied);
        assert_eq!(repeat.event_hash, first.event_hash);
        assert_eq!(repeat.version, 1);
        let mut changed = seed.clone();
        changed.cash = Money::from_cny(200_000.0).unwrap();
        assert!(matches!(
            ledger.apply(PaperCommand::Seed(changed)),
            Err(LedgerError::IdentityConflict)
        ));
        let mut connection = db.get_conn().unwrap();
        diesel::sql_query("INSERT INTO ledger(date,total_value,cash,market_value) VALUES ('2026-09-14',900000,900000,0)")
            .execute(&mut connection).unwrap();
        let view = ledger.read(&binding).unwrap();
        assert_eq!(view.cash, Money::from_cny(100_000.0).unwrap());
        assert_eq!(view.equity().unwrap(), Money::from_cny(100_000.0).unwrap());
        assert_eq!(view.fees, Money::ZERO);
        assert!(view.daily_pnl().is_none());
    }
    let reopened = DatabaseManager::open_isolated_for_test(path).unwrap();
    let view = PaperLedger::open(&reopened, &instant)
        .read(&binding)
        .unwrap();
    assert_eq!(view.cash, Money::from_cny(100_000.0).unwrap());
    assert_eq!(view.version, 1);
}

#[test]
fn paper_ledger_two_buys_partial_sell_and_next_day_mark() {
    use std::sync::atomic::{AtomicI64, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_paper_lifecycle.db"))
            .unwrap();
    let now = AtomicI64::new(instant().timestamp());
    let clock = || Utc.timestamp_opt(now.load(Ordering::SeqCst), 0).unwrap();
    let ledger = PaperLedger::open(&db, &clock);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let buy1 = order(&ledger, &binding, "buy-1", Direction::Buy, 10.0, clock());
    assert_eq!(
        ledger.apply(PaperCommand::Execute(buy1)).unwrap().status,
        LedgerStatus::Filled
    );
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, Money::from_cny(98_995.0).unwrap());
    assert_eq!(view.equity().unwrap(), Money::from_cny(99_995.0).unwrap());
    let buy2 = order(&ledger, &binding, "buy-2", Direction::Buy, 12.0, clock());
    ledger.apply(PaperCommand::Execute(buy2)).unwrap();
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, Money::from_cny(97_790.0).unwrap());
    assert_eq!(view.equity().unwrap(), Money::from_cny(100_190.0).unwrap());
    let same_day_sell = order(&ledger, &binding, "t0-sell", Direction::Sell, 11.0, clock());
    assert_eq!(
        ledger
            .apply(PaperCommand::Execute(same_day_sell))
            .unwrap()
            .status,
        LedgerStatus::Rejected
    );
    assert_eq!(ledger.read(&binding).unwrap().cash, view.cash);
    now.store(instant().timestamp() + 5 * 3600, Ordering::SeqCst);
    mark_close(&ledger, &binding, "close-d0", 12.0, clock());
    now.store(instant().timestamp() + 86400, Ordering::SeqCst);
    let sell = order(&ledger, &binding, "sell-1", Direction::Sell, 11.0, clock());
    assert_eq!(
        ledger.apply(PaperCommand::Execute(sell)).unwrap().status,
        LedgerStatus::Filled
    );
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, Money::from_cny(98_883.90).unwrap());
    assert_eq!(view.equity().unwrap(), Money::from_cny(99_983.90).unwrap());
    assert_eq!(view.fees, Money::from_cny(16.10).unwrap());
    assert_eq!(view.realized_pnl, Money::from_cny(88.90).unwrap());
    assert_eq!(view.lots.len(), 1);
    assert_eq!(view.lots[0].quantity, 100);
    assert_eq!(
        view.lots[0].buy_fee_remaining,
        Money::from_cny(5.0).unwrap()
    );
    now.store(instant().timestamp() + 86400 + 5 * 3600, Ordering::SeqCst);
    mark_close(&ledger, &binding, "close-d1", 13.0, clock());
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, Money::from_cny(98_883.90).unwrap());
    assert_eq!(view.equity().unwrap(), Money::from_cny(100_183.90).unwrap());
    assert_eq!(view.daily_pnl(), Some(Money::from_cny(-6.10).unwrap()));
}

#[test]
fn paper_ledger_idempotency_reopens_before_stale_version_or_quote_checks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_paper_idempotency.db");
    let seed = manifest();
    let binding = seed.binding().unwrap();
    let (intent, first) = {
        let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
        let ledger = PaperLedger::open(&db, &instant);
        ledger.apply(PaperCommand::Seed(seed)).unwrap();
        let intent = order(
            &ledger,
            &binding,
            "stable-buy",
            Direction::Buy,
            10.0,
            instant(),
        );
        let first = ledger.apply(PaperCommand::Execute(intent.clone())).unwrap();
        (intent, first)
    };
    let db = DatabaseManager::open_isolated_for_test(path).unwrap();
    let later = || instant() + chrono::Duration::days(1);
    let ledger = PaperLedger::open(&db, &later);
    for i in 0..10 {
        let mut repeated = intent.clone();
        if i > 0 {
            repeated.command_id = format!("new-attempt-{i}");
        }
        let result = ledger.apply(PaperCommand::Execute(repeated)).unwrap();
        assert!(result.already_applied);
        assert_eq!(result.event_hash, first.event_hash);
        assert_eq!(result.paper_trade_id, first.paper_trade_id);
    }
    let mut conflict = intent;
    conflict.signal.quantity = 200;
    assert!(matches!(
        ledger.apply(PaperCommand::Execute(conflict)),
        Err(LedgerError::IdentityConflict)
    ));
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.version, 2);
    assert_eq!(view.cash, Money::from_cny(98_995.0).unwrap());
    assert_eq!(view.fees, Money::from_cny(5.0).unwrap());
    assert_eq!(view.lots[0].quantity, 100);
}

#[test]
fn paper_ledger_idempotency_recovers_unknown_committed_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_paper_unknown.db"))
        .unwrap();
    let mut ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let intent = order(
        &ledger,
        &binding,
        "unknown-buy",
        Direction::Buy,
        10.0,
        instant(),
    );
    ledger.after_commit_fault = Some(&|| true);
    assert!(matches!(
        ledger.apply(PaperCommand::Execute(intent.clone())),
        Err(LedgerError::CommitOutcomeUnknown)
    ));
    ledger.after_commit_fault = None;
    let recovered = ledger.apply(PaperCommand::Execute(intent)).unwrap();
    assert!(recovered.already_applied);
    assert_eq!(recovered.status, LedgerStatus::Filled);
    assert_eq!(
        ledger.read(&binding).unwrap().cash,
        Money::from_cny(98_995.0).unwrap()
    );
}

#[test]
fn paper_ledger_concurrent_market_intent_replays_changed_quote_but_fixed_price_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_paper_price_intent.db"))
            .unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let mut market = order(&ledger, &binding, "market", Direction::Buy, 10.0, instant());
    market.price_intent = PriceIntent::SignalQuoteMarketV1;
    let first = ledger.apply(PaperCommand::Execute(market.clone())).unwrap();
    market.command_id = "market-next-quote".into();
    market.signal.price = 12.0;
    market.quote_price = Money::from_cny(12.0).unwrap();
    market.signal.quote_observed_at = instant() - chrono::Duration::days(1);
    let replayed = ledger.apply(PaperCommand::Execute(market)).unwrap();
    assert!(replayed.already_applied);
    assert_eq!(replayed.event_hash, first.event_hash);
    let fixed = order(&ledger, &binding, "fixed", Direction::Buy, 10.0, instant());
    ledger.apply(PaperCommand::Execute(fixed.clone())).unwrap();
    let mut changed = fixed;
    changed.signal.price = 12.0;
    assert!(matches!(
        ledger.apply(PaperCommand::Execute(changed)),
        Err(LedgerError::IdentityConflict)
    ));
}

#[test]
fn paper_ledger_concurrent_orders_cannot_double_spend_cash_or_concentration() {
    use std::sync::{Arc, Barrier};
    for concentrated in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TEST_CODE_paper_concurrent.db");
        let db1 = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
        let db2 = DatabaseManager::open_isolated_for_test(path).unwrap();
        let ledger = PaperLedger::open(&db1, &instant);
        let mut seed = manifest();
        if concentrated {
            seed.cash = Money::from_cny(91_000.0).unwrap();
            seed.lots.push(SeedLot {
                code: "TEST_CODE_000001".into(),
                name: "fixture".into(),
                quantity: 900,
                reported_cost: None,
                sellable_from: None,
                sellability_evidence: None,
            });
            seed.marks.push(Mark {
                code: "TEST_CODE_000001".into(),
                price: Money::from_cny(10.0).unwrap(),
                observed_at: instant(),
                source: "TEST_CODE_cutover".into(),
            });
        } else {
            seed.cash = Money::from_cny(1005.0).unwrap();
            seed.original_total = seed.cash;
            seed.policy.max_position_bps = 10000;
            seed.policy.cash_floor_bps = 0;
        }
        let binding = seed.binding().unwrap();
        ledger.apply(PaperCommand::Seed(seed)).unwrap();
        let first = order(
            &ledger,
            &binding,
            "parallel-a",
            Direction::Buy,
            10.0,
            instant(),
        );
        let second = order(
            &ledger,
            &binding,
            "parallel-b",
            Direction::Buy,
            10.0,
            instant(),
        );
        let barrier = Arc::new(Barrier::new(2));
        let results = std::thread::scope(|scope| {
            let a = Arc::clone(&barrier);
            let first_db = &db1;
            let first = scope.spawn(move || {
                a.wait();
                PaperLedger::open(first_db, &instant).apply(PaperCommand::Execute(first))
            });
            let b = Arc::clone(&barrier);
            let second_db = &db2;
            let second = scope.spawn(move || {
                b.wait();
                PaperLedger::open(second_db, &instant).apply(PaperCommand::Execute(second))
            });
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(
            results
                .iter()
                .filter(|r| r.as_ref().is_ok_and(|r| r.status == LedgerStatus::Filled))
                .count(),
            1,
            "{results:?}"
        );
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(LedgerError::VersionChanged)))
                .count(),
            1,
            "{results:?}"
        );
        let fresh_attempt = order(
            &ledger,
            &binding,
            "fresh-retry",
            Direction::Buy,
            10.0,
            instant(),
        );
        assert_eq!(
            ledger
                .apply(PaperCommand::Execute(fresh_attempt))
                .unwrap()
                .status,
            LedgerStatus::Rejected
        );
        let view = ledger.read(&binding).unwrap();
        assert_eq!(
            view.cash,
            Money::from_cny(if concentrated { 89_995.0 } else { 0.0 }).unwrap()
        );
        assert_eq!(view.fees, Money::from_cny(5.0).unwrap());
        assert_eq!(
            view.lots.iter().map(|lot| lot.quantity).sum::<u32>(),
            if concentrated { 1000 } else { 100 }
        );
    }
}

#[test]
fn paper_ledger_integrity_rejects_negative_seed_residual() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_negative_residual.db"))
            .unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let mut seed = manifest();
    seed.original_total = Money::from_cny(99_999.0).unwrap();
    seed.excluded_residual = Some(Money::from_cny(-1.0).unwrap());
    let binding = seed.binding().unwrap();
    assert!(matches!(
        ledger.apply(PaperCommand::Seed(seed)),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(ledger.read(&binding), Err(LedgerError::NotSeeded)));
}

#[test]
fn paper_ledger_integrity_failures_roll_back_trade_audit_event_and_cash() {
    for trigger in [
        "CREATE TRIGGER TEST_CODE_fault BEFORE INSERT ON order_audit_chain BEGIN SELECT RAISE(ABORT,'TEST_CODE_audit_failure'); END",
        "CREATE TRIGGER TEST_CODE_fault BEFORE INSERT ON paper_ledger_event BEGIN SELECT RAISE(ABORT,'TEST_CODE_event_failure'); END",
        "CREATE TRIGGER TEST_CODE_fault BEFORE UPDATE ON paper_ledger_head BEGIN SELECT RAISE(IGNORE); END",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_atomic_failure.db")).unwrap();
        let ledger = PaperLedger::open(&db,&instant);
        let seed = manifest();
        let binding = seed.binding().unwrap();
        ledger.apply(PaperCommand::Seed(seed)).unwrap();
        let before = ledger.read(&binding).unwrap();
        let intent = order(&ledger,&binding,"rollback",Direction::Buy,10.0,instant());
        let mut conn = db.get_conn().unwrap();
        conn.batch_execute(trigger).unwrap();
        assert!(ledger.apply(PaperCommand::Execute(intent.clone())).is_err());
        assert_eq!(ledger.read(&binding).unwrap(),before);
        for table in ["paper_trades","order_audit","order_audit_chain"] {
            let count = diesel::sql_query(format!("SELECT COUNT(*) AS value FROM {table}")).get_result::<IntegerRow>(&mut conn).unwrap().value;
            assert_eq!(count,0,"orphan in {table}");
        }
        conn.batch_execute("DROP TRIGGER TEST_CODE_fault").unwrap();
        let applied = ledger.apply(PaperCommand::Execute(intent)).unwrap();
        assert_eq!(applied.status,LedgerStatus::Filled);
        assert_eq!(ledger.read(&binding).unwrap().version,2);
    }
}

#[test]
fn paper_ledger_integrity_cache_loss_is_explicitly_repaired_not_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_cache_repair.db"))
        .unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "buy",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    let before = ledger.read(&binding).unwrap();
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query("DELETE FROM paper_ledger_head")
        .execute(&mut conn)
        .unwrap();
    assert!(matches!(
        ledger.read(&binding),
        Err(LedgerError::IntegrityFailure(_))
    ));
    assert_eq!(ledger.repair_missing_projection(&binding).unwrap(), before);
    assert_eq!(ledger.read(&binding).unwrap(), before);
    diesel::sql_query("UPDATE paper_ledger_head SET projection_hash='tampered'")
        .execute(&mut conn)
        .unwrap();
    assert!(matches!(
        ledger.read(&binding),
        Err(LedgerError::IntegrityFailure(_))
    ));
    assert!(matches!(
        ledger.repair_missing_projection(&binding),
        Err(LedgerError::IntegrityFailure(_))
    ));
    assert!(
        diesel::sql_query("UPDATE paper_ledger_event SET payload='tampered'")
            .execute(&mut conn)
            .is_err()
    );
    conn.batch_execute("DROP TRIGGER paper_ledger_event_no_update; UPDATE paper_ledger_event SET payload='tampered' WHERE seq=2").unwrap();
    assert!(matches!(
        ledger.read(&binding),
        Err(LedgerError::IntegrityFailure(_))
    ));
}

#[test]
fn paper_ledger_integrity_cancel_and_lock_delay_do_not_authorize_stale_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_cancel_busy.db");
    let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
    let competing_db = DatabaseManager::open_isolated_for_test(path).unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let intent = order(
        &ledger,
        &binding,
        "cancelled",
        Direction::Buy,
        10.0,
        instant(),
    );
    let cancelled = AtomicBool::new(false);
    let cancel_at_lock = || {
        cancelled.store(true, Ordering::SeqCst);
        instant()
    };
    assert!(matches!(
        PaperLedger::open(&db, &cancel_at_lock)
            .apply_controlled(PaperCommand::Execute(intent.clone()), &cancelled),
        Err(LedgerError::Cancelled)
    ));
    let mut writer = db.get_conn().unwrap();
    writer.batch_execute("BEGIN IMMEDIATE").unwrap();
    let result =
        PaperLedger::open(&competing_db, &instant).apply(PaperCommand::Execute(intent.clone()));
    writer.batch_execute("ROLLBACK").unwrap();
    assert!(
        matches!(result, Err(LedgerError::BusyRetryable)),
        "{result:?}"
    );
    let after_wait = || instant() + chrono::Duration::seconds(6);
    assert!(matches!(
        PaperLedger::open(&db, &after_wait).apply(PaperCommand::Execute(intent)),
        Err(LedgerError::EvidenceUnavailable(_))
    ));
    assert_eq!(ledger.read(&binding).unwrap().version, 1);
}

#[test]
fn paper_ledger_runtime_terminal_recovery_precedes_all_quote_io_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_runtime_recovery.db");
    let seed = manifest();
    let binding = seed.binding().unwrap();
    let quote = crate::broker::ExecutionQuote {
        price: 10.0,
        limit_up_price: 11.0,
        limit_down_price: 9.0,
        observed_at: instant(),
    };
    let cancelled = AtomicBool::new(false);
    let (mut signal, before) = {
        let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
        let ledger = PaperLedger::open(&db, &instant);
        ledger.apply(PaperCommand::Seed(seed)).unwrap();
        let mut signal = order(
            &ledger,
            &binding,
            "pushed-stock-buy-v1:recover",
            Direction::Buy,
            10.0,
            instant(),
        )
        .signal;
        signal.plan_id = "pushed-stock-buy-v1:recover".into();
        let source = |_: &str| Ok(quote.clone());
        let first = crate::trading::paper_ledger_runtime::execute_on(
            &db, &binding, &signal, &quote, &instant, &source, &cancelled,
        )
        .unwrap();
        assert!(first.inserted);
        let mut other = signal.clone();
        other.plan_id = "pushed-stock-buy-v1:other".into();
        other.code = "TEST_CODE_OTHER_HOLDING".into();
        crate::trading::paper_ledger_runtime::execute_on(
            &db, &binding, &other, &quote, &instant, &source, &cancelled,
        )
        .unwrap();
        (signal, ledger.read(&binding).unwrap())
    };
    let reopened = DatabaseManager::open_isolated_for_test(path).unwrap();
    let later = || instant() + chrono::Duration::days(1);
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let unavailable = |_: &str| {
        calls.fetch_add(1, Ordering::SeqCst);
        Err("TEST_CODE all external quotes unavailable".into())
    };
    // Both supplied evidence and the external source are now unusable. A
    // committed market-plan receipt is still independent of them.
    signal.price = 12.0;
    let again = crate::trading::paper_ledger_runtime::execute_on(
        &reopened,
        &binding,
        &signal,
        &quote,
        &later,
        &unavailable,
        &cancelled,
    )
    .expect("recover committed receipt without any quote I/O");
    assert!(!again.inserted);
    assert_eq!(again.result.fill_price, Some(10.0));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    signal.quantity += 100;
    let conflict = crate::trading::paper_ledger_runtime::execute_on(
        &reopened,
        &binding,
        &signal,
        &quote,
        &later,
        &unavailable,
        &cancelled,
    )
    .unwrap_err();
    assert!(conflict.contains("identity conflict"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    signal.plan_id = format!("paper:{}:{}", binding.epoch_id, signal.plan_id);
    assert!(matches!(
        PaperLedger::open(&reopened, &later).recover_terminal(
            &binding,
            &signal,
            PriceIntent::SignalQuoteMarketV1,
        ),
        Err(LedgerError::IdentityConflict)
    ));
    assert_eq!(
        PaperLedger::open(&reopened, &later).read(&binding).unwrap(),
        before
    );
}

#[test]
fn paper_ledger_runtime_binding_and_epoch_isolate_legacy_fills_and_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_runtime_epoch.db"))
        .unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed.clone())).unwrap();
    let mut signal = order(
        &ledger,
        &binding,
        "pushed-stock-buy-v1:42",
        Direction::Buy,
        10.0,
        instant(),
    )
    .signal;
    signal.plan_id = "pushed-stock-buy-v1:42".into();
    let quote = crate::broker::ExecutionQuote {
        price: 10.0,
        limit_up_price: 11.0,
        limit_down_price: 9.0,
        observed_at: instant(),
    };
    let source = |_: &str| Ok(quote.clone());
    let cancelled = AtomicBool::new(false);
    let mut conn = db.get_conn().unwrap();
    conn.batch_execute("INSERT INTO paper_trades(plan_id,code,name,direction,price,quantity,ts,status,fill_price,virtual_reason,account_mode,data_mode) VALUES ('pushed-stock-buy-v1:42','TEST_CODE_000001','legacy','sell',10,900,'2026-09-13 10:00:00','Filled',10,'TEST_CODE_legacy','Normal','Full')").unwrap();
    let first = crate::trading::paper_ledger_runtime::execute_on(
        &db, &binding, &signal, &quote, &instant, &source, &cancelled,
    )
    .unwrap();
    assert!(first.inserted);
    let mut changed = signal.clone();
    changed.price = 12.0;
    let again = crate::trading::paper_ledger_runtime::execute_on(
        &db, &binding, &changed, &quote, &instant, &source, &cancelled,
    )
    .unwrap();
    assert!(!again.inserted);
    assert_eq!(again.result.fill_price, Some(10.0));
    let mut second_seed = seed;
    second_seed.account_id = "TEST_CODE_SECOND_ACCOUNT".into();
    second_seed.epoch_id = "TEST_CODE_SECOND_EPOCH".into();
    let second = second_seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(second_seed)).unwrap();
    crate::trading::paper_ledger_runtime::execute_on(
        &db, &second, &signal, &quote, &instant, &source, &cancelled,
    )
    .unwrap();
    for binding in [&binding, &second] {
        let view = ledger.read(binding).unwrap();
        assert_eq!(view.cash, Money::from_cny(98_995.0).unwrap());
        assert_eq!(view.lots[0].quantity, 100);
        assert_eq!(view.fees, Money::from_cny(5.0).unwrap());
    }
    let absent = AccountBinding {
        account_id: "TEST_CODE_UNSEEDED".into(),
        ..binding
    };
    assert!(crate::trading::paper_ledger_runtime::execute_on(
        &db, &absent, &signal, &quote, &instant, &source, &cancelled
    )
    .is_err());
    let count = diesel::sql_query("SELECT COUNT(*) AS value FROM paper_trades")
        .get_result::<IntegerRow>(&mut conn)
        .unwrap()
        .value;
    assert_eq!(
        count, 3,
        "original legacy fill must remain alongside two isolated epochs"
    );
}

#[test]
fn paper_ledger_history_read_points_and_effective_fill_seam_share_one_chain() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_history.db")).unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let genesis = ledger.read(&binding).unwrap();
    let receipt = ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "buy",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    assert_eq!(ledger.read_at_version(&binding, 1).unwrap(), genesis);
    assert_eq!(
        ledger.read_at_version(&binding, 2).unwrap(),
        ledger.read(&binding).unwrap()
    );
    assert!(ledger.read_at_version(&binding, 0).is_err());
    assert!(ledger.read_at_version(&binding, 3).is_err());
    let fills = ledger.effective_fills(&binding).unwrap();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].paper_trade_id, receipt.paper_trade_id.unwrap());
    assert_eq!(fills[0].event_hash, receipt.event_hash);
    assert_eq!(fills[0].fill_price, Money::from_cny(10.0).unwrap());
    assert_eq!(fills[0].fee, Money::from_cny(5.0).unwrap());
    assert_eq!(
        ledger.read(&binding).unwrap().unrealized_pnl().unwrap(),
        Money::from_cny(-5.0).unwrap()
    );
}

#[test]
fn paper_ledger_history_terminal_nonfills_marks_and_explicit_rejected_retry() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_terminals.db")).unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    for invalidated in [false, true] {
        let mut intent = order(
            &ledger,
            &binding,
            if invalidated { "slip" } else { "halt" },
            Direction::Buy,
            10.0,
            instant(),
        );
        intent.price_intent = PriceIntent::SignalQuoteMarketV1;
        if invalidated {
            intent.quote_price = Money::from_cny(11.0).unwrap();
        } else {
            intent.signal.is_suspended = true;
        }
        let receipt = ledger.apply(PaperCommand::Execute(intent.clone())).unwrap();
        assert_eq!(
            receipt.status,
            if invalidated {
                LedgerStatus::Invalidated
            } else {
                LedgerStatus::NotFilled
            }
        );
        intent.command_id.push_str("retry");
        intent.signal.is_suspended = false;
        intent.signal.quote_observed_at = instant() - chrono::Duration::days(1);
        assert_eq!(
            ledger
                .apply(PaperCommand::Execute(intent))
                .unwrap()
                .event_hash,
            receipt.event_hash
        );
    }
    let mut rejected = order(
        &ledger,
        &binding,
        "rejected",
        Direction::Buy,
        10.0,
        instant(),
    );
    rejected.signal.risk_context = PaperRiskContext::new(
        crate::risk::action_gate::AccountMode::Normal,
        crate::monitor::data_mode::DataMode::Unsafe,
    );
    let receipt = ledger
        .apply(PaperCommand::Execute(rejected.clone()))
        .unwrap();
    assert_eq!(receipt.status, LedgerStatus::Rejected);
    rejected.signal.risk_context = PaperRiskContext::new(
        crate::risk::action_gate::AccountMode::Normal,
        crate::monitor::data_mode::DataMode::Full,
    );
    assert_eq!(
        ledger
            .apply(PaperCommand::Execute(rejected.clone()))
            .unwrap()
            .event_hash,
        receipt.event_hash
    );
    rejected.command_id.push_str("explicit-new-attempt");
    rejected.expected_version = ledger.read(&binding).unwrap().version;
    assert_eq!(
        ledger
            .apply(PaperCommand::Execute(rejected))
            .unwrap()
            .status,
        LedgerStatus::Filled
    );
    let before = ledger.read(&binding).unwrap();
    let batch = ValuationBatch {
        binding: binding.clone(),
        command_id: "mark".into(),
        expected_version: before.version,
        inventory_fingerprint: before.inventory_fingerprint().unwrap(),
        as_of: instant(),
        closing: false,
        marks: vec![Mark {
            code: "TEST_CODE_000001".into(),
            price: Money::from_cny(12.0).unwrap(),
            observed_at: instant(),
            source: "TEST_CODE_mark".into(),
        }],
    };
    let mark = ledger.apply(PaperCommand::Mark(batch.clone())).unwrap();
    assert_eq!(
        ledger.apply(PaperCommand::Mark(batch)).unwrap().event_hash,
        mark.event_hash
    );
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, before.cash);
    assert_eq!(view.lots, before.lots);
    assert!(view.daily_pnl().is_none());
    let mut missing = order(
        &ledger,
        &binding,
        "missing",
        Direction::Buy,
        10.0,
        instant(),
    );
    missing.marks.clear();
    assert!(matches!(
        ledger.apply(PaperCommand::Execute(missing)),
        Err(LedgerError::EvidenceUnavailable(_))
    ));
    assert_eq!(ledger.read(&binding).unwrap(), view);
}

#[test]
fn paper_ledger_history_exposure_reducing_sell_ignores_buy_only_floors() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_sell_floor.db"))
        .unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let mut seed = manifest();
    seed.cash = Money::ZERO;
    seed.original_total = Money::from_cny(1000.0).unwrap();
    seed.lots = vec![SeedLot {
        code: "TEST_CODE_000001".into(),
        name: "fixture".into(),
        quantity: 100,
        reported_cost: Some(Money::from_cny(20.0).unwrap()),
        sellable_from: Some(day(instant())),
        sellability_evidence: Some("TEST_CODE_confirmed_inventory".into()),
    }];
    seed.marks = vec![Mark {
        code: "TEST_CODE_000001".into(),
        price: Money::from_cny(10.0).unwrap(),
        observed_at: instant(),
        source: "TEST_CODE_cutover".into(),
    }];
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let sell = order(
        &ledger,
        &binding,
        "reduce",
        Direction::Sell,
        10.0,
        instant(),
    );
    assert_eq!(
        ledger.apply(PaperCommand::Execute(sell)).unwrap().status,
        LedgerStatus::Filled
    );
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, Money::from_cny(994.0).unwrap());
    assert_eq!(
        view.realized_pnl,
        Money::from_cny(-6.0).unwrap(),
        "reported historical cost must not become post-cutover loss"
    );
    assert!(view.lots.is_empty());
    assert!(crate::trading::paper_trade::simulate(
        &order(
            &ledger,
            &binding,
            "disabled",
            Direction::Buy,
            10.0,
            instant()
        )
        .signal,
        10.0,
        1_000_000.0,
        1_000_000.0,
        0.0
    )
    .unwrap_err()
    .contains("disabled"));
}

#[test]
fn paper_ledger_history_partial_lot_absorbs_fee_remainder_and_overflow_rolls_back() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_fee_remainder.db"))
        .unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let excessive = order(
        &ledger,
        &binding,
        "overflow",
        Direction::Buy,
        100_000_000_000.0,
        instant(),
    );
    assert!(matches!(
        ledger.apply(PaperCommand::Execute(excessive)),
        Err(LedgerError::Overflow)
    ));
    assert_eq!(ledger.read(&binding).unwrap().version, 1);
    let mut buy = order(
        &ledger,
        &binding,
        "buy-300",
        Direction::Buy,
        10.0,
        instant(),
    );
    buy.signal.quantity = 300;
    ledger.apply(PaperCommand::Execute(buy)).unwrap();
    let tomorrow = || instant() + chrono::Duration::days(1);
    let ledger = PaperLedger::open(&db, &tomorrow);
    for (index, remaining_fee) in [(1, 3.333334), (2, 1.666667), (3, 0.0)] {
        let sell = order(
            &ledger,
            &binding,
            &format!("sell-{index}"),
            Direction::Sell,
            10.0,
            tomorrow(),
        );
        assert_eq!(
            ledger.apply(PaperCommand::Execute(sell)).unwrap().status,
            LedgerStatus::Filled
        );
        let view = ledger.read(&binding).unwrap();
        assert_eq!(
            view.lots
                .first()
                .map(|lot| lot.buy_fee_remaining)
                .unwrap_or(Money::ZERO),
            Money::from_cny(remaining_fee).unwrap()
        );
    }
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, Money::from_cny(99_977.0).unwrap());
    assert_eq!(view.realized_pnl, Money::from_cny(-23.0).unwrap());
    assert_eq!(view.fees, Money::from_cny(23.0).unwrap());
}
